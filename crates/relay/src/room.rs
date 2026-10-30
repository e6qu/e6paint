//! Per-room realtime actor: owns the live room doc, fans out batches and presence,
//! persists periodic snapshots.

use std::sync::Arc;
use std::time::Duration;

use loro::LoroDoc;
use tokio::sync::{mpsc, oneshot};

/// Boxed snapshot-persist callback type.
type PersistCallback = Arc<dyn Fn(&[u8]) -> Result<(), String> + Send + Sync>;

/// WS binary payloads carry a one-byte tag: `1` = board update batch (Loro updates),
/// `2` = presence JSON, `3` = snapshot.
pub const TAG_BATCH: u8 = 1;
pub const TAG_PRESENCE: u8 = 2;
pub const TAG_SNAPSHOT: u8 = 3;
pub const TAG_WELCOME: u8 = 4;
pub const TAG_PING: u8 = 5;
pub const TAG_PONG: u8 = 6;

/// Opaque per-connection identity (uuid v4 minted by the WS handler).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerId(pub Arc<String>);

impl PeerId {
    pub fn mint() -> Self {
        Self(Arc::new(uuid::Uuid::new_v4().to_string()))
    }
}

/// Messages the connection tasks send into the room actor.
pub enum RoomMsg {
    /// A peer connected; carries its access level.
    Hello { peer: PeerId, read_only: bool },
    /// A peer sent a board update batch.
    Batch { peer: PeerId, bytes: Vec<u8> },
    /// A peer sent ephemeral presence JSON.
    Presence { peer: PeerId, json: String },
    /// A joining peer requests the current snapshot.
    Snapshot { reply: oneshot::Sender<Vec<u8>> },
}

/// How many imported batches trigger a deep compaction by default (fresh doc rebuilt
/// from the exported snapshot, dropping superseded history from memory). The relay
/// binary reads `E6PAINT_COMPACT_EVERY` and hands the value to the room actor
/// explicitly; this constant is only the unset-env default.
pub const DEFAULT_COMPACT_EVERY_BATCHES: usize = 500;

/// Rebuild `doc`'s content into a fresh document, dropping superseded oplog history
/// from memory. Peer identities and state are preserved (snapshot import keeps both).
pub fn compact_doc(doc: &LoroDoc) -> Result<LoroDoc, String> {
    let snapshot = doc
        .export(loro::ExportMode::Snapshot)
        .map_err(|err| err.to_string())?;
    let fresh = LoroDoc::new();
    fresh.import(&snapshot).map_err(|err| err.to_string())?;
    Ok(fresh)
}

/// Handles a connection task holds to talk to the room actor and hear broadcasts.
#[derive(Clone)]
pub struct RoomHandles {
    pub ingest: mpsc::Sender<RoomMsg>,
    pub broadcast: tokio::sync::broadcast::Sender<(PeerId, u8, Vec<u8>)>,
}

/// Persist callback, boxed once per actor.
#[derive(Clone)]
pub struct PersistFn(pub PersistCallback);

/// Spawn the room actor. `snapshot` is the persisted snapshot (empty for a fresh
/// room); `compact_every` is the deep-compaction cadence in imported batches. The
/// actor exits — persisting first when dirty — once the ingest channel closes, i.e.
/// when the last client leaves and the registry drops its handle.
pub fn spawn(
    mut ingest: mpsc::Receiver<RoomMsg>,
    broadcast: tokio::sync::broadcast::Sender<(PeerId, u8, Vec<u8>)>,
    snapshot: Vec<u8>,
    persist: PersistFn,
    compact_every: usize,
    room_id: String,
) {
    tokio::spawn(async move {
        let mut doc = LoroDoc::new();
        if !snapshot.is_empty()
            && let Err(err) = doc.import(&snapshot)
        {
            log::error!("room snapshot failed to import: {err}");
        }
        let mut dirty = false;
        let mut batches_since_compact = 0usize;
        // Access level per connected peer: batches from read-only peers are rejected.
        let mut read_only_peers: std::collections::HashSet<PeerId> =
            std::collections::HashSet::new();
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                msg = ingest.recv() => match msg {
                    Some(RoomMsg::Hello { peer, read_only }) => {
                        log::info!(
                            "peer {} joined as {}",
                            peer.0,
                            if read_only { "read-only" } else { "read-write" }
                        );
                        if read_only {
                            read_only_peers.insert(peer);
                        } else {
                            read_only_peers.remove(&peer);
                        }
                    }
                    Some(RoomMsg::Batch { peer, bytes }) => {
                        if read_only_peers.contains(&peer) {
                            log::warn!("batch from read-only peer {} rejected", peer.0);
                            continue;
                        }
                        match doc.import(&bytes) {
                            Ok(status) => {
                                log::info!(
                                    "batch from {} imported ({status:?}); broadcasting",
                                    peer.0
                                );
                                // Compact element geometry after every import:
                                // the e2e specs assert board state from this
                                // line instead of parsing snapshots.
                                let elements = doc.get_map("elements").get_deep_value();
                                log::info!(
                                    "DOCDBG room={} {}",
                                    room_id,
                                    serde_json::to_string(&elements).unwrap_or_default()
                                );
                                dirty = true;
                                batches_since_compact += 1;
                                let _ = broadcast.send((peer, TAG_BATCH, bytes));
                            }
                            Err(err) => {
                                log::error!("batch from {} rejected: {err}", peer.0);
                            }
                        }
                    }
                    Some(RoomMsg::Presence { peer, json }) => {
                        let _ = broadcast.send((peer, TAG_PRESENCE, json.into_bytes()));
                    },
                    Some(RoomMsg::Snapshot { reply }) => match export_snapshot(&doc) {
                        Ok(bytes) => {
                            let _ = reply.send(wrap(TAG_SNAPSHOT, &bytes));
                        }
                        Err(err) => {
                            log::error!("snapshot export failed: {err}");
                            let _ = reply.send(wrap(TAG_SNAPSHOT, &[]));
                        }
                    },
                    None => {
                        if dirty
                            && let Err(err) = persist_snapshot(&doc, &persist)
                        {
                            log::error!("final snapshot persist failed: {err}");
                        }
                        break;
                    }
                },
                _ = ticker.tick(), if dirty => {
                    if let Err(err) = persist_snapshot(&doc, &persist) {
                        log::error!("periodic snapshot persist failed: {err}");
                    }
                    dirty = false;
                }
            }
            if batches_since_compact >= compact_every {
                match compact_doc(&doc) {
                    Ok(fresh) => {
                        doc = fresh;
                        log::info!("room doc compacted after {batches_since_compact} batches");
                    }
                    Err(err) => log::error!("room doc compaction failed: {err}"),
                }
                batches_since_compact = 0;
            }
        }
    });
}

fn export_snapshot(doc: &LoroDoc) -> Result<Vec<u8>, String> {
    doc.export(loro::ExportMode::Snapshot)
        .map_err(|err| err.to_string())
}

fn persist_snapshot(doc: &LoroDoc, persist: &PersistFn) -> Result<(), String> {
    let bytes = export_snapshot(doc)?;
    persist.0.as_ref()(&bytes)
}

fn wrap(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(tag);
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    #[test]
    fn compaction_preserves_state() -> Result<(), Box<dyn std::error::Error>> {
        let doc = LoroDoc::new();
        let map = doc.get_map("elements");
        let _ = map.insert("k", "v");
        doc.commit();

        let fresh = compact_doc(&doc)?;
        assert_eq!(
            fresh.get_map("elements").get_deep_value(),
            doc.get_map("elements").get_deep_value(),
            "state survives the rebuild"
        );
        assert_eq!(
            fresh.oplog_vv(),
            doc.oplog_vv(),
            "version identity preserved"
        );
        Ok(())
    }
}
