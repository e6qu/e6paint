//! Relay binary entry. Persists to `E6PAINT_RELAY_DB` (default `./e6paint-relay.db`),
//! binds `E6PAINT_RELAY_ADDR` (default `127.0.0.1:8480`), deep-compacts room docs
//! every `E6PAINT_COMPACT_EVERY` imported batches (default 500), and logs to stderr
//! (`RUST_LOG`, default `warn` per env_logger).

use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let addr: std::net::SocketAddr = match std::env::var("E6PAINT_RELAY_ADDR") {
        Ok(raw) => raw.parse()?,
        Err(_) => std::net::SocketAddr::from(([127, 0, 0, 1], 8480)),
    };
    let db_path = match std::env::var("E6PAINT_RELAY_DB") {
        Ok(raw) => PathBuf::from(raw),
        Err(_) => PathBuf::from("e6paint-relay.db"),
    };
    let compact_every = match std::env::var("E6PAINT_COMPACT_EVERY") {
        Ok(raw) => raw.parse::<usize>()?,
        Err(_) => e6_relay::room::DEFAULT_COMPACT_EVERY_BATCHES,
    };
    let state = Arc::new(e6_relay::RelayState::open(db_path, compact_every)?);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("e6paint relay listening on {addr} (compacting every {compact_every} batches)");
    axum::serve(listener, e6_relay::router(state)).await?;
    Ok(())
}
