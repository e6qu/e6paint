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

    /// Wrap a rotation to [0, 2pi), snapping near-zero values to exactly 0 so
    /// full-turn round trips stay byte-identical to an unrotated element.
    pub fn normalize_rotation(rotation: f32) -> f32 {
        let tau = core::f32::consts::TAU;
        let wrapped = rotation.rem_euclid(tau);
        if wrapped < 0.01 || tau - wrapped < 0.01 {
            0.0
        } else {
            wrapped
        }
    }

    /// Map a world point into this element's local (unrotated) frame: the inverse
    /// of the rotation about the bounds center.
    pub fn to_local(&self, world: Point) -> Point {
        if self.rotation == 0.0 {
            return world;
        }
        let center = self.bounds.center();
        let (sin, cos) = f64::from(-self.rotation).sin_cos();
        let dx = f64::from(world.x) - f64::from(center.x);
        let dy = f64::from(world.y) - f64::from(center.y);
        Point::new(
            ((dx * cos - dy * sin) + f64::from(center.x)) as f32,
            ((dx * sin + dy * cos) + f64::from(center.y)) as f32,
        )
    }

    /// The four bounds corners rotated about the bounds center (the outline a
    /// rotated element paints).
    pub fn rotated_corners(&self) -> [Point; 4] {
        if self.rotation == 0.0 {
            return [
                self.bounds.min,
                Point::new(self.bounds.max.x, self.bounds.min.y),
                self.bounds.max,
                Point::new(self.bounds.min.x, self.bounds.max.y),
            ];
        }
        let center = self.bounds.center();
        let (sin, cos) = f64::from(self.rotation).sin_cos();
        let (hw, hh) = (
            f64::from(self.bounds.size().width) / 2.0,
            f64::from(self.bounds.size().height) / 2.0,
        );
        let (cx, cy) = (f64::from(center.x), f64::from(center.y));
        [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(dx, dy)| {
            let (rx, ry) = (dx * cos - dy * sin, dx * sin + dy * cos);
            Point::new((cx + rx) as f32, (cy + ry) as f32)
        })
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
    fn rotation_normalizes_full_turns_to_zero() {
        use core::f32::consts::{FRAC_PI_2, TAU};
        assert_eq!(Element::normalize_rotation(0.0), 0.0);
        assert_eq!(Element::normalize_rotation(TAU), 0.0);
        assert_eq!(Element::normalize_rotation(-TAU), 0.0);
        assert_eq!(
            Element::normalize_rotation(24.0 * (core::f32::consts::PI / 12.0)),
            0.0
        );
        let quarter = Element::normalize_rotation(FRAC_PI_2);
        assert!((quarter - FRAC_PI_2).abs() < 1e-6);
        let three_quarter = Element::normalize_rotation(-FRAC_PI_2);
        assert!((three_quarter - 3.0 * FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn to_local_inverts_the_rotation_about_the_center() {
        use core::f32::consts::FRAC_PI_2;
        let bounds = Aabb::from_min_size(Point::new(100.0, 200.0), Size::new(80.0, 40.0));
        let element = Element {
            rotation: FRAC_PI_2,
            ..Element::new(
                ElementId::from_raw(1),
                ElementKind::Rect,
                bounds,
                0xFF00_00FF,
            )
        };
        let center = bounds.center();
        // A point on the painted top edge maps back onto the local top edge.
        let painted_top = Point::new(center.x - 10.0, center.y - 40.0);
        let local = element.to_local(painted_top);
        assert!(
            bounds.contains(local),
            "painted point {painted_top:?} maps into local bounds, got {local:?}"
        );
        // The center is its own image.
        let local_center = element.to_local(center);
        assert!((local_center.x - center.x).abs() < 1e-4);
        assert!((local_center.y - center.y).abs() < 1e-4);
    }

    #[test]
    fn rotated_corners_swap_extents_for_a_quarter_turn() {
        use core::f32::consts::FRAC_PI_2;
        let bounds = Aabb::from_min_size(Point::new(0.0, 0.0), Size::new(100.0, 50.0));
        let element = Element {
            rotation: FRAC_PI_2,
            ..Element::new(
                ElementId::from_raw(1),
                ElementKind::Rect,
                bounds,
                0xFF00_00FF,
            )
        };
        let corners = element.rotated_corners();
        let center = bounds.center();
        // Quarter turn: the 100x50 shape paints 50 wide by 100 tall, so corner
        // offsets from the center are 25 across and 50 up/down.
        for corner in corners {
            assert!((corner.x - center.x).abs() - 25.0 < 1e-4);
            assert!((corner.y - center.y).abs() - 50.0 < 1e-4);
        }
        // Unrotated elements return their axis-aligned corners unchanged.
        let flat = Element::new(
            ElementId::from_raw(2),
            ElementKind::Rect,
            bounds,
            0xFF00_00FF,
        );
        let corners = flat.rotated_corners();
        assert_eq!(corners[0], bounds.min);
        assert_eq!(corners[2], bounds.max);
    }

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
