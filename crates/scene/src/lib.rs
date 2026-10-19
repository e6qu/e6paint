//! Scene model: elements, geometry, camera, spatial index.
//!
//! Pure core: no IO, no logging, no threads, no `panic!`. Errors are `Option`/`Result`
//! values returned to the caller.

pub mod camera;
pub mod demo;
pub mod element;
pub mod geom;
pub mod index;

pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM};
pub use demo::{demo_board, torture_board};
pub use element::{Element, ElementId, ElementKind, ImageAsset};
pub use geom::{Aabb, Point, Size};
pub use index::{NaiveIndex, SpatialIndex};
