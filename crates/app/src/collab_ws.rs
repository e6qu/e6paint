//! WebSocket collaboration transport (web): room creation over fetch, then a live
//! socket whose events land in an inbox the app drains each frame. Closures are kept
//! alive by the [`Transport`] handle. Native builds get a no-op stub for now — the
//! launch target is web (DESIGN D4).

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;

/// A joined room. `read_token` is present only when we created the room ourselves
/// (the create response hands it back); joiners via share links never see it, which
/// is exactly why view links can only be minted by the room's creator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomConnection {
    pub room_id: String,
    pub token: String,
    pub read_token: Option<String>,
}

/// Socket events, drained by the app each frame.
#[derive(Debug, Clone)]
pub enum WsEvent {
    Open,
    /// Tagged binary frame (relay protocol).
    Frame(Vec<u8>),
    Closed,
}

#[cfg(target_arch = "wasm32")]
/// Live transport. Dropping it closes the socket (closures die with it).
pub struct Transport {
    socket: web_sys::WebSocket,
    _onmessage: wasm_bindgen::closure::Closure<dyn FnMut(wasm_bindgen::JsValue)>,
    _onopen: wasm_bindgen::closure::Closure<dyn FnMut()>,
    _onclose: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(target_arch = "wasm32")]
/// In-flight join flow: creates a room, then connects.
pub struct JoinFlow {
    events: Arc<Mutex<Vec<WsEvent>>>,
    /// `Rc<RefCell>`, not `Arc/Mutex`: the transport carries wasm closures (never
    /// `Send`), and wasm32 runs single-threaded.
    ready: Rc<RefCell<Option<Transport>>>,
    room: Arc<Mutex<Option<RoomConnection>>>,
}

#[cfg(target_arch = "wasm32")]
#[cfg(target_arch = "wasm32")]
impl JoinFlow {
    /// Start the join flow against `relay_http_base` (e.g. `http://127.0.0.1:8480`).
    /// The WS URL is derived from it (http→ws).
    pub fn start(relay_http_base: &str) -> Self {
        let events: Arc<Mutex<Vec<WsEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let ready: Rc<RefCell<Option<Transport>>> = Rc::new(RefCell::new(None));
        let room: Arc<Mutex<Option<RoomConnection>>> = Arc::new(Mutex::new(None));

        let window = web_sys::window();
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        let request_result =
            web_sys::Request::new_with_str_and_init(&format!("{relay_http_base}/rooms"), &init);

        match (window, request_result) {
            (Some(window), Ok(request)) => {
                let promise = JsFuture::from(window.fetch_with_request(&request));
                let events = events.clone();
                let ready = ready.clone();
                let room = room.clone();
                let relay_http_base = relay_http_base.to_owned();
                wasm_bindgen_futures::spawn_local(async move {
                    match join_via_fetch(promise).await {
                        Ok(connection) => {
                            let ws_url = to_ws_url(&relay_http_base, &connection);
                            match Transport::connect(&ws_url, events.clone()) {
                                Ok(transport) => {
                                    if let Ok(mut slot) = room.lock() {
                                        *slot = Some(connection);
                                    }
                                    *ready.borrow_mut() = Some(transport);
                                }
                                Err(err) => log::error!("socket connect failed: {err}"),
                            }
                        }
                        Err(err) => log::error!("room creation failed: {err}"),
                    }
                });
            }
            (None, _) => log::error!("no window; sharing is unavailable"),
            (_, Err(err)) => log::error!("room request build failed: {err:?}"),
        }

        Self {
            events,
            ready,
            room,
        }
    }

    /// Connect straight to an existing room (share-link flow): no room creation, same
    /// tagged-frame protocol. Room/transport land in the same slots as `start`.
    pub fn connect_to(relay_http_base: &str, room_id: &str, token: &str) -> Self {
        let events: Arc<Mutex<Vec<WsEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let ready: Rc<RefCell<Option<Transport>>> = Rc::new(RefCell::new(None));
        // Link joiners hold only the handed-out token; the read token stays with the
        // room's creator.
        let room: Arc<Mutex<Option<RoomConnection>>> = Arc::new(Mutex::new(Some(RoomConnection {
            room_id: room_id.to_owned(),
            token: token.to_owned(),
            read_token: None,
        })));

        let connection = RoomConnection {
            room_id: room_id.to_owned(),
            token: token.to_owned(),
            read_token: None,
        };
        let ws_url = to_ws_url(relay_http_base, &connection);
        match Transport::connect(&ws_url, events.clone()) {
            Ok(transport) => {
                *ready.borrow_mut() = Some(transport);
            }
            Err(err) => log::error!("socket connect failed: {err}"),
        }

        Self {
            events,
            ready,
            room,
        }
    }

    /// The joined room, once the flow completes.
    pub fn take_room(&self) -> Option<RoomConnection> {
        self.room.lock().ok().and_then(|mut slot| slot.take())
    }

    /// The live transport, once the socket opens.
    pub fn take_transport(&self) -> Option<Transport> {
        self.ready.borrow_mut().take()
    }

    /// Snapshot of queued socket events.
    pub fn drain_events(&self) -> Vec<WsEvent> {
        self.events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }
}

#[cfg(target_arch = "wasm32")]
async fn join_via_fetch(promise: wasm_bindgen_futures::JsFuture) -> Result<RoomConnection, String> {
    let response = promise.await.map_err(|err| format!("{err:?}"))?;
    let response: web_sys::Response = response.into();
    if !response.ok() {
        return Err(format!("room creation returned {}", response.status()));
    }
    let json_future = response
        .json()
        .map_err(|err| format!("response.json failed: {err:?}"))?;
    let text = json_future.await.map_err(|err| format!("{err:?}"))?;
    let text = text.as_string().ok_or("room json was not text")?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let room_id = value["room_id"]
        .as_str()
        .ok_or("missing room_id")?
        .to_owned();
    let token = value["token"].as_str().ok_or("missing token")?.to_owned();
    // The relay hands back the read token on create; without it the creator could
    // never mint view-only links, so a missing field is a protocol error.
    let read_token = value["read_token"]
        .as_str()
        .ok_or("missing read_token")?
        .to_owned();
    Ok(RoomConnection {
        room_id,
        token,
        read_token: Some(read_token),
    })
}

#[cfg(target_arch = "wasm32")]
pub fn to_ws_url(relay_http_base: &str, connection: &RoomConnection) -> String {
    let ws_base = relay_http_base.replacen("http", "ws", 1);
    format!(
        "{ws_base}/ws/{}?token={}",
        connection.room_id, connection.token
    )
}

#[cfg(target_arch = "wasm32")]
#[cfg(target_arch = "wasm32")]
impl Transport {
    /// Open a socket; socket events are pushed into `events`.
    pub fn connect(ws_url: &str, events: Arc<Mutex<Vec<WsEvent>>>) -> Result<Self, String> {
        let socket = web_sys::WebSocket::new(ws_url).map_err(|err| format!("{err:?}"))?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let onmessage = {
            let events = events.clone();
            wasm_bindgen::closure::Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(
                move |js: wasm_bindgen::JsValue| {
                    let Ok(event) = js.dyn_into::<web_sys::MessageEvent>() else {
                        return;
                    };
                    let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
                        return;
                    };
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    if let Ok(mut queue) = events.lock() {
                        queue.push(WsEvent::Frame(bytes));
                    }
                },
            )
        };
        let onopen = {
            let events = events.clone();
            wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                if let Ok(mut queue) = events.lock() {
                    queue.push(WsEvent::Open);
                }
            })
        };
        let onclose = {
            let events = events.clone();
            wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                if let Ok(mut queue) = events.lock() {
                    queue.push(WsEvent::Closed);
                }
            })
        };
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        Ok(Self {
            socket,
            _onmessage: onmessage,
            _onopen: onopen,
            _onclose: onclose,
        })
    }

    /// Send one tagged frame.
    pub fn send(&self, frame: Vec<u8>) -> Result<(), String> {
        self.socket
            .send_with_u8_array(&frame)
            .map_err(|err| format!("{err:?}"))
    }

    /// Whether the socket is still open.
    pub fn is_open(&self) -> bool {
        self.socket.ready_state() == web_sys::WebSocket::OPEN
    }
}
