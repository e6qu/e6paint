//! PNG rasterization worker: runs [`e6_render::rasterize_svg_png`] off the
//! main thread so exporting a dense board never freezes the page.
//!
//! Protocol (postMessage): `{ svg: string, scale: number }` in,
//! `{ png: Uint8Array }` out on success, `{ error: string }` on failure.

use wasm_bindgen::prelude::*;

/// Rasterize an exported-board SVG into PNG bytes at the given scale.
#[wasm_bindgen]
pub fn rasterize_svg_png(svg: &str, scale: u32) -> Result<Vec<u8>, JsValue> {
    e6_render::rasterize_svg_png(svg, scale).map_err(|err| JsValue::from_str(&err))
}
