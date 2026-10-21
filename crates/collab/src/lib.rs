//! Local-first sync glue: presence, send throttling, and the transport port.
//!
//! Presence is ephemeral — it never enters the CRDT. The WebSocket transport itself is
//! shell territory; core defines only the port and the pacing policy around it.

use web_time::{Duration, Instant};

use e6_scene::{ElementId, Point};

/// Ephemeral participant state broadcast over the presence channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Presence {
    pub name: String,
    /// Participant tint, `0xAARRGGBB`.
    pub color: u32,
    pub cursor: Point,
    pub selection: Vec<ElementId>,
}

/// Gates high-frequency sends (cursor moves) to a fixed tick.
///
/// Cloned state shares nothing — each sender owns its own gate.
#[derive(Debug, Clone)]
pub struct SendThrottle {
    min_interval: Duration,
    last_sent: Option<Instant>,
}

/// Default pacing for cursor-presence sends (~30 Hz ceiling).
pub const CURSOR_SEND_INTERVAL: Duration = Duration::from_millis(33);

impl Default for SendThrottle {
    fn default() -> Self {
        Self {
            min_interval: CURSOR_SEND_INTERVAL,
            last_sent: None,
        }
    }
}

impl SendThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_sent: None,
        }
    }

    /// `true` (and arms the gate) if a send is due at `now`. The first call always sends.
    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last_sent {
            Some(sent) if now.duration_since(sent) < self.min_interval => false,
            _ => {
                self.last_sent = Some(now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn throttle_sends_first_then_respects_interval() {
        let start = Instant::now();
        let mut throttle = SendThrottle::new(Duration::from_millis(100));

        assert!(throttle.ready(start), "first send is always due");
        assert!(!throttle.ready(start + Duration::from_millis(50)));
        assert!(throttle.ready(start + Duration::from_millis(101)));
        assert!(!throttle.ready(start + Duration::from_millis(150)));
        assert!(throttle.ready(start + Duration::from_millis(202)));
    }
}

/// WS binary frame tags — wire-compatible with the relay protocol.
pub mod frame {
    /// Board update batch (Loro updates).
    pub const TAG_BATCH: u8 = 1;
    /// Presence JSON.
    pub const TAG_PRESENCE: u8 = 2;
    /// Snapshot (join payload).
    pub const TAG_SNAPSHOT: u8 = 3;
    /// Server -> peer, right after the join snapshot: `{"read_only": bool}`.
    pub const TAG_WELCOME: u8 = 4;
    /// Client -> server liveness probe; the server echoes TAG_PONG.
    pub const TAG_PING: u8 = 5;
    /// Server -> client, echo of TAG_PING with identical payload.
    pub const TAG_PONG: u8 = 6;

    /// Wrap a payload in its tag.
    pub fn wrap(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 1);
        out.push(tag);
        out.extend_from_slice(payload);
        out
    }

    /// Strip the tag, returning `(tag, payload)`.
    pub fn unwrap(frame: &[u8]) -> Option<(u8, &[u8])> {
        frame.split_first().map(|(tag, rest)| (*tag, rest))
    }
}

/// This client's identity on a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    /// Stable per-session peer key (relayed to others).
    pub peer: String,
    pub name: String,
    /// Cursor tint, `0xAARRGGBB`.
    pub color: u32,
}

/// Presence state broadcast to peers (ephemeral, never persisted).
#[derive(Debug, Clone, PartialEq)]
pub struct PresenceUpdate {
    pub identity: ClientIdentity,
    pub cursor: Point,
}

/// The presence wire form: primitives only, so scene types stay out of the
/// protocol. Serialization and parsing share this one definition.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct PresenceWire {
    peer: String,
    name: String,
    /// Cursor tint as `#rrggbb` (alpha dropped — it is a local-only channel).
    color: String,
    x: f32,
    y: f32,
}

/// Serialize a presence frame payload (the JSON inside a presence frame).
pub fn presence_json(update: &PresenceUpdate) -> String {
    let wire = PresenceWire {
        peer: update.identity.peer.clone(),
        name: update.identity.name.clone(),
        color: format!("#{:06x}", update.identity.color & 0xFF_FFFF),
        x: update.cursor.x,
        y: update.cursor.y,
    };
    serde_json::to_string(&wire).unwrap_or_default()
}

/// Parse a presence frame payload. Malformed frames are `Err` — the caller
/// reports them; nothing silently defaults.
pub fn presence_from_json(bytes: &[u8]) -> Result<PresenceUpdate, String> {
    let wire: PresenceWire =
        serde_json::from_slice(bytes).map_err(|err| format!("malformed presence: {err}"))?;
    let color = u32::from_str_radix(wire.color.trim_start_matches('#'), 16)
        .map_err(|err| format!("malformed presence color `{}`: {err}", wire.color))?;
    Ok(PresenceUpdate {
        identity: ClientIdentity {
            peer: wire.peer,
            name: wire.name,
            color,
        },
        cursor: Point::new(wire.x, wire.y),
    })
}

/// Outgoing sync batches. Every batch is full history, so a newer batch fully
/// supersedes an older unsent one — the queue collapses to at most one entry.
#[derive(Debug, Default)]
pub struct Outbox {
    pending: Option<Vec<u8>>,
}

impl Outbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue (or replace) the unsent batch.
    pub fn push(&mut self, batch: Vec<u8>) {
        self.pending = Some(batch);
    }

    /// Take the queued batch, if any.
    pub fn take(&mut self) -> Option<Vec<u8>> {
        self.pending.take()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_none()
    }
}

/// Exponential reconnect backoff: 250 ms doubling to a 10 s cap.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    attempt: u32,
}

/// A ping is considered unanswered when it went out at least `timeout` ago and
/// no pong arrived after it. No ping outstanding means the connection is
/// presumed healthy (the heartbeat sends one on its interval).
pub fn pong_timed_out(
    last_ping: Option<Instant>,
    last_pong: Option<Instant>,
    now: Instant,
    timeout: Duration,
) -> bool {
    let Some(sent) = last_ping else {
        return false;
    };
    if now.duration_since(sent) < timeout {
        return false;
    }
    last_pong.is_none_or(|pong| pong < sent)
}

/// First retry delay.
pub const FIRST_RETRY: Duration = Duration::from_millis(250);
/// Retry delay ceiling.
pub const MAX_RETRY: Duration = Duration::from_secs(10);

impl Backoff {
    /// Record a failed attempt and return the next delay.
    pub fn fail(&mut self) -> Duration {
        let delay = FIRST_RETRY
            .checked_mul(1_u32 << self.attempt.min(6))
            .unwrap_or(MAX_RETRY)
            .min(MAX_RETRY);
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Reset after a successful connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn presence_json_carries_identity_and_cursor() {
        let json = presence_json(&PresenceUpdate {
            identity: ClientIdentity {
                peer: "p1".into(),
                name: "Ada".into(),
                color: 0xFF33_66CC,
            },
            cursor: Point::new(12.5, 3.0),
        });
        let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(value["peer"], "p1");
        assert_eq!(value["name"], "Ada");
        assert_eq!(value["color"], "#3366cc");
        assert_eq!(value["x"], 12.5);
        assert_eq!(value["y"], 3.0);
    }

    #[test]
    fn outbox_collapses_to_the_newest_batch() {
        let mut outbox = Outbox::new();
        assert!(outbox.is_empty());
        outbox.push(vec![1]);
        outbox.push(vec![2, 3]);
        assert_eq!(outbox.take(), Some(vec![2, 3]));
        assert!(outbox.is_empty());
        assert_eq!(outbox.take(), None);
    }

    #[test]
    fn backoff_grows_then_caps_and_resets() {
        let mut backoff = Backoff::default();
        assert_eq!(backoff.fail(), Duration::from_millis(250));
        assert_eq!(backoff.fail(), Duration::from_millis(500));
        assert_eq!(backoff.fail(), Duration::from_millis(1000));
        for _ in 0..12 {
            let delay = backoff.fail();
            assert!(delay <= MAX_RETRY);
        }
        backoff.reset();
        assert_eq!(backoff.fail(), FIRST_RETRY);
    }

    #[test]
    fn frame_tags_wrap_and_unwrap() {
        let frame = frame::wrap(frame::TAG_BATCH, &[9, 9]);
        assert_eq!(frame::unwrap(&frame), Some((frame::TAG_BATCH, &[9, 9][..])));
        assert_eq!(frame::unwrap(&[]), None);
    }
}

/// One `key=value` pair from a query string, trimmed and dropped when empty.
fn query_param(query: &str, wanted: &str) -> Option<String> {
    let query = query.trim_start_matches('?');
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next().map(str::trim) != Some(wanted) {
            continue;
        }
        let value = parts.next().unwrap_or_default().trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// Parsed share-link query parameters: `?room=…&token=…` (relay base comes from the
/// app's own location/config, not the link, so links stay portable across hosts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLink {
    pub room: String,
    pub token: String,
}

/// Extract share parameters from a URL query string (`?room=x&token=y`).
/// Returns `None` unless both parameters are present and non-empty.
pub fn parse_share_query(query: &str) -> Option<ShareLink> {
    Some(ShareLink {
        room: query_param(query, "room")?,
        token: query_param(query, "token")?,
    })
}

/// The `?relay=` override from a query string, if present and non-empty. Scheme
/// validation is the caller's job.
pub fn relay_override(query: &str) -> Option<String> {
    query_param(query, "relay")
}

#[cfg(test)]
mod share_tests {
    use super::*;

    #[test]
    fn presence_json_round_trips_through_the_wire_form() -> Result<(), Box<dyn std::error::Error>> {
        let update = PresenceUpdate {
            identity: ClientIdentity {
                peer: "p1".into(),
                name: "Ada".into(),
                color: 0xFF33_66CC,
            },
            cursor: Point::new(12.5, 3.0),
        };
        let bytes = presence_json(&update).into_bytes();
        let parsed = presence_from_json(&bytes)?;
        // Alpha never travels (it is a local-only channel), so the tint comes
        // back as the 6-digit RGB the sender broadcast.
        assert_eq!(parsed.identity.peer, "p1");
        assert_eq!(parsed.identity.name, "Ada");
        assert_eq!(parsed.identity.color, 0x33_66CC);
        assert_eq!(parsed.cursor, Point::new(12.5, 3.0));
        Ok(())
    }

    #[test]
    fn pong_timeout_detects_only_answered_liveness() {
        let now = web_time::Instant::now();
        let sent = now - web_time::Duration::from_secs(20);
        assert!(pong_timed_out(
            Some(sent),
            None,
            now,
            web_time::Duration::from_secs(15)
        ));
        let pong_before_ping = sent - web_time::Duration::from_secs(1);
        assert!(pong_timed_out(
            Some(sent),
            Some(pong_before_ping),
            now,
            web_time::Duration::from_secs(15)
        ));
        assert!(!pong_timed_out(
            Some(now - web_time::Duration::from_secs(2)),
            None,
            now,
            web_time::Duration::from_secs(15)
        ));
        let pong_after_ping = now;
        assert!(!pong_timed_out(
            Some(now - web_time::Duration::from_secs(2)),
            Some(pong_after_ping),
            now,
            web_time::Duration::from_secs(15)
        ));
        assert!(!pong_timed_out(
            None,
            None,
            now,
            web_time::Duration::from_secs(15)
        ));
    }

    #[test]
    fn malformed_presence_is_an_error_not_a_default() {
        assert!(presence_from_json(b"{not json").is_err());
        assert!(presence_from_json(br#"{"peer":"p"}"#).is_err());
    }

    #[test]
    fn relay_override_extracts_and_requires_value() {
        assert_eq!(
            relay_override("?room=r&token=t&relay=http://relay.example:8480").as_deref(),
            Some("http://relay.example:8480")
        );
        assert_eq!(relay_override("?relay="), None);
        assert_eq!(relay_override("?room=r"), None);
        assert_eq!(relay_override(""), None);
    }

    #[test]
    fn parses_room_and_token() -> Result<(), Box<dyn std::error::Error>> {
        let link = parse_share_query("?room=r-1&token=t-1").ok_or("both params")?;
        assert_eq!(link.room, "r-1");
        assert_eq!(link.token, "t-1");
        Ok(())
    }

    #[test]
    fn tolerates_order_whitespace_and_extras() -> Result<(), Box<dyn std::error::Error>> {
        let link = parse_share_query("?x=1&  token = t  &room = r ").ok_or("both params")?;
        assert_eq!(link.room, "r");
        assert_eq!(link.token, "t");
        Ok(())
    }

    #[test]
    fn missing_either_param_is_none() {
        assert!(parse_share_query("?room=r").is_none());
        assert!(parse_share_query("?token=t").is_none());
        assert!(parse_share_query("").is_none());
    }
}
