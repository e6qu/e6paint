//! e6paint relay: WebSocket rooms, token auth, snapshot persistence.
//!
//! Clients exchange tagged binary frames (see [`room`]) and ephemeral presence JSON.
//! The relay holds a live [`loro::LoroDoc`] per active room: incoming batches are
//! imported, fanned out to the other peers, and folded into periodic snapshots
//! persisted to SQLite. Joiners receive the current snapshot first. The wire protocol
//! is the only coupling to the app.

pub mod room;
pub mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use room::{
    PeerId, PersistFn, RoomHandles, RoomMsg, TAG_BATCH, TAG_PING, TAG_PONG, TAG_PRESENCE,
    TAG_SNAPSHOT, TAG_WELCOME,
};

/// Shared relay state: SQLite handle + live room registry.
pub struct RelayState {
    db_path: PathBuf,
    db: Mutex<store::Connection>,
    rooms: Mutex<HashMap<String, RoomHandles>>,
    compact_every: usize,
}

impl RelayState {
    /// Opens (creating) the database at `db_path`; `compact_every` is the room-doc
    /// deep-compaction cadence handed to every room actor this state spawns.
    pub fn open(db_path: PathBuf, compact_every: usize) -> Result<Self, String> {
        let db = store::open(&db_path)?;
        Ok(Self {
            db_path,
            db: Mutex::new(db),
            rooms: Mutex::new(HashMap::new()),
            compact_every,
        })
    }

    /// Database path (exposed for logging at startup).
    pub fn db_path(&self) -> &std::path::Path {
        &self.db_path
    }
}

/// The relay's HTTP/WS surface.
pub fn router(state: Arc<RelayState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/rooms", post(create_room))
        .route("/ws/{room_id}", get(ws_upgrade))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        concat!("e6paint relay v", env!("CARGO_PKG_VERSION"), "\n"),
    )
}

/// POST /rooms: mint a room with write and read-only tokens. The JSON response
/// carries `room_id`, `token`, `read_token` — exactly the fields the web
/// client's join flow requires (the DAG keeps the two sides decoupled; this
/// comment and the e2e are the contract).
async fn create_room(
    State(state): State<Arc<RelayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let room_id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let read_token = uuid::Uuid::new_v4().to_string();
    {
        let db = state.db.lock().map_err(|_| internal("room db poisoned"))?;
        store::insert_room(&db, &room_id, &token, &read_token).map_err(|err| internal(&err))?;
    }
    Ok(Json(json!({
        "room_id": room_id,
        "token": token,
        "read_token": read_token,
    })))
}

fn internal(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": message })),
    )
}

#[derive(serde::Deserialize)]
struct WsQuery {
    token: String,
}

/// WS /ws/{room_id}?token=…: authenticated realtime join.
#[axum::debug_handler]
async fn ws_upgrade(
    State(state): State<Arc<RelayState>>,
    Path(room_id): Path<String>,
    Query(query): Query<WsQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, (StatusCode, Json<serde_json::Value>)> {
    // Auth: the write token grants edits, the read-only token grants presence only.
    let (persisted_snapshot, read_only) = {
        let db = state.db.lock().map_err(|_| internal("room db poisoned"))?;
        let row = store::load_room(&db, &room_id)
            .map_err(|err| internal(&err))?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "no such room" })),
                )
            })?;
        if row.token == query.token {
            (row.snapshot.unwrap_or_default(), false)
        } else if row.read_token.as_deref() == Some(query.token.as_str()) {
            (row.snapshot.unwrap_or_default(), true)
        } else {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "bad room token" })),
            ));
        }
    };

    let (handles, snapshot) = join_room(&state, &room_id, persisted_snapshot)
        .await
        .map_err(|err| internal(&err))?;
    let peer = PeerId::mint();
    handles
        .ingest
        .send(RoomMsg::Hello {
            peer: peer.clone(),
            read_only,
        })
        .await
        .map_err(|_| internal("room closed during join"))?;
    Ok(
        upgrade
            .on_upgrade(move |socket| run_connection(socket, handles, peer, snapshot, read_only)),
    )
}

/// Get (or create) the room actor's handles; the returned snapshot is the live doc's
/// current state — the single snapshot source, seeded from storage at actor spawn.
/// The registry guard is never held across an await (std Mutex is not Send).
async fn join_room(
    state: &Arc<RelayState>,
    room_id: &str,
    persisted_snapshot: Vec<u8>,
) -> Result<(RoomHandles, Vec<u8>), String> {
    let (handles, spawn_channels) = {
        let mut rooms = state
            .rooms
            .lock()
            .map_err(|_| "room registry poisoned".to_string())?;
        if let Some(handles) = rooms.get(room_id) {
            (handles.clone(), None)
        } else {
            let (ingest, ingest_rx) = mpsc::channel::<RoomMsg>(64);
            let (broadcast, _) = tokio::sync::broadcast::channel::<(PeerId, u8, Vec<u8>)>(128);
            let handles = RoomHandles {
                ingest: ingest.clone(),
                broadcast: broadcast.clone(),
            };
            rooms.insert(room_id.to_owned(), handles.clone());
            (handles, Some((ingest_rx, broadcast)))
        }
    };

    if let Some((ingest_rx, broadcast)) = spawn_channels {
        let db_path = state.db_path.clone();
        let room_id_owned = room_id.to_owned();
        let persist = PersistFn(Arc::new(move |bytes: &[u8]| {
            let db = store::open(&db_path)?;
            store::save_snapshot(&db, &room_id_owned, bytes)
        }));
        room::spawn(
            ingest_rx,
            broadcast,
            persisted_snapshot.clone(),
            persist,
            state.compact_every,
        );
    }

    let (reply, receiver) = oneshot::channel();
    handles
        .ingest
        .send(RoomMsg::Snapshot { reply })
        .await
        .map_err(|_| "room closed during join".to_string())?;
    let snapshot = receiver
        .await
        .map_err(|_| "room closed during join".to_string())?;
    Ok((handles, snapshot))
}

/// One connected peer: reader + writer halves over the tagged-frame protocol.
async fn run_connection(
    socket: WebSocket,
    handles: RoomHandles,
    peer: PeerId,
    snapshot: Vec<u8>,
    read_only: bool,
) {
    use futures_util::{SinkExt, StreamExt};

    let (mut sink, mut stream) = socket.split();
    log::info!("peer {peer:?} connected");

    // Joining peer receives the current snapshot first. The actor's reply is already a
    // tagged frame.
    if snapshot.len() > 1 && sink.send(Message::Binary(snapshot.into())).await.is_err() {
        return;
    }

    // Tell the peer its access level: a view-only client surfaces this instead of
    // silently dropping its edits on the floor.
    let welcome = wrap(
        TAG_WELCOME,
        serde_json::to_vec(&json!({ "read_only": read_only }))
            .unwrap_or_default()
            .as_slice(),
    );
    if sink.send(Message::Binary(welcome.into())).await.is_err() {
        return;
    }

    let mut broadcast_rx = handles.broadcast.subscribe();
    let writer_peer = peer.clone();
    // Connection-local outbound lane: pings echo here without touching the room
    // actor, so liveness works even while a peer is idle.
    let (local_out_tx, mut local_out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let write_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                outcome = broadcast_rx.recv() => {
                    match outcome {
                        Ok((sender, tag, payload)) => {
                            if sender == writer_peer {
                                continue;
                            }
                            if sink
                                .send(Message::Binary(wrap(tag, &payload).into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            log::warn!("peer lagged behind {count} frames; continuing");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                message = local_out_rx.recv() => {
                    match message {
                        Some(message) => {
                            if sink.send(message).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });
    let local_out_tx = std::sync::Arc::new(local_out_tx);

    let ingest = handles.ingest.clone();
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { break };
        match message {
            Message::Binary(bytes) if bytes.len() >= 2 => match bytes[0] {
                TAG_BATCH => {
                    let send = ingest.send(RoomMsg::Batch {
                        peer: peer.clone(),
                        bytes: bytes[1..].to_vec(),
                    });
                    if send.await.is_err() {
                        break;
                    }
                }
                TAG_PRESENCE => {
                    let json = String::from_utf8_lossy(&bytes[1..]).to_string();
                    let send = ingest.send(RoomMsg::Presence {
                        peer: peer.clone(),
                        json,
                    });
                    if send.await.is_err() {
                        break;
                    }
                }
                TAG_SNAPSHOT => {
                    log::warn!("peer {peer:?} sent a snapshot frame; ignored");
                }
                // Liveness is between the peer and its own connection: echoed
                // here without the room actor so dead peers are detectable even
                // while idle.
                TAG_PING => {
                    let pong = wrap(TAG_PONG, &bytes[1..]);
                    if local_out_tx.send(Message::Binary(pong.into())).is_err() {
                        break;
                    }
                }
                _ => {}
            },
            Message::Close(_) => break,
            _ => {}
        }
    }
    write_task.abort();
    log::info!("peer {peer:?} left");
}

fn wrap(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(tag);
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::body::Body;
    pub(crate) use tower::ServiceExt;

    pub(crate) fn test_state() -> Result<Arc<RelayState>, Box<dyn std::error::Error>> {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "e6relay-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        Ok(Arc::new(RelayState::open(
            path,
            room::DEFAULT_COMPACT_EVERY_BATCHES,
        )?))
    }

    #[tokio::test]
    async fn healthz_reports_ok_with_version() -> Result<(), Box<dyn std::error::Error>> {
        let request = axum::http::Request::builder()
            .uri("/healthz")
            .body(Body::empty())?;
        let response = router(test_state()?).oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    #[tokio::test]
    async fn rooms_are_created_with_tokens() -> Result<(), Box<dyn std::error::Error>> {
        let app = router(test_state()?);
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/rooms")
            .body(Body::empty())?;
        let response = app.oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await?
            .to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body)?;
        assert!(value["room_id"].as_str().is_some());
        assert!(value["token"].as_str().is_some());
        Ok(())
    }
}

#[cfg(test)]
mod ws_e2e_tests {
    use super::tests::{ServiceExt, test_state};
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use loro::{ExportMode, LoroDoc};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    /// Spawn the relay on an ephemeral port; returns (addr, Arc state).
    async fn spawn_server()
    -> Result<(std::net::SocketAddr, Arc<RelayState>), Box<dyn std::error::Error>> {
        let state = test_state()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server_state = state.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(server_state)).await;
        });
        Ok((addr, state))
    }

    #[allow(unused_variables)]
    async fn create_room(
        state: &Arc<RelayState>,
        addr: std::net::SocketAddr,
    ) -> Result<(String, String, String), Box<dyn std::error::Error>> {
        let app = router(state.clone());
        use axum::body::Body;
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("http://localhost/rooms")
            .body(Body::empty())?;
        let response = app.oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await?
            .to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body)?;
        Ok((
            value["room_id"].as_str().ok_or("room_id")?.to_owned(),
            value["token"].as_str().ok_or("token")?.to_owned(),
            value["read_token"].as_str().ok_or("read_token")?.to_owned(),
        ))
    }

    async fn connect(
        addr: std::net::SocketAddr,
        room: &str,
        token: &str,
    ) -> Result<
        (
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
            futures_util::stream::SplitStream<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
            >,
        ),
        Box<dyn std::error::Error>,
    > {
        let url = format!("ws://{addr}/ws/{room}?token={token}");
        let (socket, _) = tokio_tungstenite::connect_async(url).await?;
        Ok(socket.split())
    }

    async fn next_binary<S>(stream: &mut S) -> Result<Vec<u8>, Box<dyn std::error::Error>>
    where
        S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + std::marker::Unpin,
    {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await?
                .ok_or("stream ended before a frame arrived")?;
            match message? {
                Message::Binary(data) => return Ok(data.to_vec()),
                Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_) => {}
                Message::Close(_) => return Err("peer closed".into()),
            }
        }
    }

    /// A small board change encoded as a full-history batch (the proven protocol).
    fn sample_batch(marker: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use loro::LoroDoc;
        let doc = LoroDoc::new();
        let map = doc.get_map("proof");
        let _ = map.insert(marker, marker);
        doc.commit();
        Ok(doc.export(ExportMode::all_updates())?)
    }

    #[tokio::test]
    async fn two_peers_exchange_snapshot_batches_and_presence()
    -> Result<(), Box<dyn std::error::Error>> {
        let (addr, state) = spawn_server().await?;
        let (room_id, token, read_token) = create_room(&state, addr).await?;

        // Peer A joins; every joiner (even into a fresh room) receives the live doc
        // snapshot first.
        let (mut a_tx, mut a_rx) = connect(addr, &room_id, &token).await?;
        let join_snapshot = next_binary(&mut a_rx).await?;
        assert_eq!(join_snapshot[0], TAG_SNAPSHOT);
        let join_welcome = next_binary(&mut a_rx).await?;
        assert_eq!(
            join_welcome[0], TAG_WELCOME,
            "joiners learn their access level"
        );

        // Liveness: a ping is echoed as a pong with identical payload.
        a_tx.send(Message::Binary(wrap(TAG_PING, br#"probe"#).into()))
            .await?;
        let pong = next_binary(&mut a_rx).await?;
        assert_eq!(pong[0], TAG_PONG);
        assert_eq!(&pong[1..], br#"probe"#);

        a_tx.send(Message::Binary(
            wrap(TAG_BATCH, &sample_batch("from-a")?).into(),
        ))
        .await?;

        // Peer B joins later and receives a snapshot containing A's batch.
        let (mut b_tx, mut b_rx) = connect(addr, &room_id, &token).await?;
        let snapshot = next_binary(&mut b_rx).await?;
        assert_eq!(snapshot[0], TAG_SNAPSHOT, "joiners get a snapshot first");
        let welcome = next_binary(&mut b_rx).await?;
        assert_eq!(welcome[0], TAG_WELCOME, "joiners learn their access level");
        let mirror = LoroDoc::new();
        mirror.import(&snapshot[1..])?;
        assert!(
            format!("{:?}", mirror.get_map("proof").get_deep_value()).contains("from-a"),
            "snapshot carries A's marker"
        );

        // B pushes its own batch; A receives it fanned out.
        b_tx.send(Message::Binary(
            wrap(TAG_BATCH, &sample_batch("from-b")?).into(),
        ))
        .await?;
        let fanout = next_binary(&mut a_rx).await?;
        assert_eq!(fanout[0], TAG_BATCH);
        assert!(String::from_utf8_lossy(&fanout[1..]).contains("from-b"));

        // Presence is forwarded to the other peer only.
        a_tx.send(Message::Binary(
            wrap(TAG_PRESENCE, br#"{"peer":"a","x":1}"#).into(),
        ))
        .await?;
        let presence = next_binary(&mut b_rx).await?;
        assert_eq!(presence[0], TAG_PRESENCE);
        assert!(String::from_utf8_lossy(&presence[1..]).contains("\"peer\":\"a\""));

        // Bad tokens are rejected at upgrade time.
        let bad =
            tokio_tungstenite::connect_async(format!("ws://{addr}/ws/{room_id}?token=nope")).await;
        assert!(bad.is_err(), "wrong token must not upgrade");

        // Read-only peer: joins fine, presence routes, but batches are rejected.
        let (mut ro_tx, mut ro_rx) = connect(addr, &room_id, &read_token).await?;
        let ro_snapshot = next_binary(&mut ro_rx).await?;
        assert_eq!(ro_snapshot[0], TAG_SNAPSHOT);
        ro_tx
            .send(Message::Binary(
                wrap(TAG_BATCH, &sample_batch("from-ro")?).into(),
            ))
            .await?;
        ro_tx
            .send(Message::Binary(
                wrap(TAG_PRESENCE, br#"{"peer":"ro"}"#).into(),
            ))
            .await?;
        let presence = next_binary(&mut a_rx).await?;
        assert_eq!(presence[0], TAG_PRESENCE);
        assert!(String::from_utf8_lossy(&presence[1..]).contains("\"peer\":\"ro\""));
        // A's next frame is B's presence echo order-safe? No: A receives ro presence
        // only if ro→A broadcast; assert A's board never sees the ro batch by
        // re-reading with a short timeout is flaky, so verify via B's snapshot instead:
        let (c_tx, mut c_rx) = connect(addr, &room_id, &token).await?;
        let fresh = next_binary(&mut c_rx).await?;
        assert_eq!(fresh[0], TAG_SNAPSHOT);
        let mirror = LoroDoc::new();
        mirror.import(&fresh[1..])?;
        assert!(
            !format!("{:?}", mirror.get_map("proof").get_deep_value()).contains("from-ro"),
            "read-only batches must not merge"
        );
        let _ = c_tx;
        Ok(())
    }
}
