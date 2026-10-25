//! Scene elements: the closed set of things a board can hold.

use crate::geom::{Aabb, Point};

/// Typed, client-minted element identifier; never reused within a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(u64);

impl ElementId {
    /// Wrap a raw handle. Opaque; minted by the document layer.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Kind of a scene element. Closed union — adding a kind is a deliberate, compile-time event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    /// Axis-aligned rectangle (also the LOD placeholder shape).
    Rect,
    /// Ellipse inscribed in the element bounds.
    Ellipse,
    /// Freehand stroke path (payload in `points`).
    Stroke,
    /// Text block (payload in `text`).
    Text,
    /// Sticky note (`text` on a rounded, colored card).
    Sticky,
    /// Raster image.
    Image,
    /// Named region and present-mode waypoint.
    Frame,
    /// Filled polygon (pie slices); payload in `points` (closed automatically).
    Sector,
    /// Connector between two anchors (`points[0]` → `points[1]`, arrowhead at the end).
    Connector,
}

/// A placed element: identity, kind, world bounds, style, and kind payload.
///
/// One struct with per-kind payload fields keeps the document mapping and the renderer
/// total over a single closed type; fields a kind doesn't use stay at their defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: ElementId,
    pub kind: ElementKind,
    pub bounds: Aabb,
    /// Fill color, `0xAARRGGBB`.
    pub color: u32,
    /// Outline color, `0xAARRGGBB` (shapes and connectors).
    pub stroke_color: u32,
    /// Outline width in world units; `0` for filled-only shapes.
    pub stroke_width: f32,
    /// Freehand stroke polyline, or `[start, end]` for connectors, in world space.
    pub points: Vec<Point>,
    /// Text/sticky content.
    pub text: String,
    /// Font size in world units for text/sticky.
    pub font_size: f32,
    /// Group membership tag; elements sharing a tag move and select together.
    pub group: Option<String>,
    /// Frame membership tag; set when the element's center is inside a frame.
    pub frame: Option<String>,
    /// Locked elements are skipped by canvas hit-testing until unlocked.
    pub locked: bool,
    /// Hidden elements are invisible and unselectable on canvas; the layers panel
    /// toggles them.
    pub hidden: bool,
    /// Rotation in radians, counter-clockwise about the bounds center.
    pub rotation: f32,
    /// Decoded image payload (Arc'd so per-frame board clones stay cheap).
    pub image: Option<ImageAsset>,
    /// Generator provenance key (e.g. `mol:0:gate`), used by re-apply to match
    /// regenerated content against its source statement.
    pub provenance: Option<String>,
}

/// Decoded, straight-alpha RGBA8 pixels for an [`ElementKind::Image`].
#[derive(Clone, Debug, PartialEq)]
pub struct ImageAsset {
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<Vec<u8>>,
}

impl Element {
    /// A filled element without payload (rect/ellipse/frame/image defaults).
    pub fn new(id: ElementId, kind: ElementKind, bounds: Aabb, color: u32) -> Self {
        Self {
            id,
            kind,
            bounds,
            color,
            stroke_color: color,
            stroke_width: 0.0,
            points: Vec::new(),
            text: String::new(),
            font_size: 24.0,
            rotation: 0.0,
            group: None,
            frame: None,
            locked: false,
            hidden: false,
            image: None,
            provenance: None,
        }
    }

    /// Builder: outline style (shapes drawn stroked, not filled).
    pub fn outlined(mut self, stroke_color: u32, stroke_width: f32) -> Self {
        self.stroke_color = stroke_color;
        self.stroke_width = stroke_width;
        self
    }

    /// Builder: polyline payload (strokes, connectors).
    pub fn with_points(mut self, points: Vec<Point>) -> Self {
        self.points = points;
        self
    }

    /// Builder: text payload.
    pub fn with_text(mut self, text: String, font_size: f32) -> Self {
        self.text = text;
        self.font_size = font_size;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;

    #[test]
    fn defaults_are_filled_and_payload_free() {
        let element = Element::new(
            ElementId::from_raw(1),
            ElementKind::Rect,
            Aabb::from_min_size(Point::ZERO, Size::new(10.0, 10.0)),
            0xFF00_00FF,
        );
        assert_eq!(element.stroke_color, element.color);
        assert_eq!(element.stroke_width, 0.0);
        assert!(element.points.is_empty());
        assert!(element.text.is_empty());
    }

    #[test]
    fn builders_stack() {
        let element = Element::new(
            ElementId::from_raw(1),
            ElementKind::Connector,
            Aabb::from_min_size(Point::ZERO, Size::new(10.0, 10.0)),
            0xFF00_00FF,
        )
        .with_points(vec![Point::new(0.0, 0.0), Point::new(10.0, 10.0)])
        .outlined(0xFFFF_0000, 2.0);
        assert_eq!(element.points.len(), 2);
        assert_eq!(element.stroke_width, 2.0);
    }
}
