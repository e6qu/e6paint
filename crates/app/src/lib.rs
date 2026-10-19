//! e6paint application shell: eframe + egui + Vello composition.
//!
//! This shell crate owns every impure boundary: GPU, window, DOM, and storage. Core logic
//! stays in the core crates.

pub mod app;
pub mod collab_ws;
pub mod gfx;
pub mod storage;

pub use app::E6App;
