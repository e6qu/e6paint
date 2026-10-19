//! Viewport camera: world↔screen mapping with zoom-at-point and pan.

use crate::geom::{Aabb, Point, Size};

/// Minimum zoom: screen pixels per world unit, zoomed far out.
pub const MIN_ZOOM: f32 = 0.02;
/// Maximum zoom: screen pixels per world unit, zoomed far in.
pub const MAX_ZOOM: f32 = 64.0;

/// Orthographic camera over the world plane.
///
/// Screen space is the render target's *physical pixel* space; world space is the board
/// plane in abstract units. Callers translate egui point-space input before use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// World position of the viewport's top-left corner.
    pub origin: Point,
    /// Screen pixels per world unit, clamped to `[MIN_ZOOM, MAX_ZOOM]`.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            origin: Point::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn screen_to_world(&self, screen: Point) -> Point {
        Point::new(
            self.origin.x + screen.x / self.zoom,
            self.origin.y + screen.y / self.zoom,
        )
    }

    pub fn world_to_screen(&self, world: Point) -> Point {
        Point::new(
            (world.x - self.origin.x) * self.zoom,
            (world.y - self.origin.y) * self.zoom,
        )
    }

    /// The visible world region for a viewport of `viewport` screen pixels.
    pub fn viewport_aabb(&self, viewport: Size) -> Aabb {
        Aabb {
            min: self.origin,
            max: self.screen_to_world(Point::new(viewport.width, viewport.height)),
        }
    }

    /// Move the view by a screen-space delta; positive delta means content follows the pointer.
    pub fn pan_by(&mut self, screen_delta: Point) {
        self.origin.x -= screen_delta.x / self.zoom;
        self.origin.y -= screen_delta.y / self.zoom;
    }

    /// Zoom by `factor` around `screen`, keeping the world point under it fixed.
    ///
    /// Non-finite or non-positive factors leave the camera unchanged: callers pass
    /// accumulated input factors that can degenerate on some devices, and a degenerate
    /// factor is input sanitation (like clamping), not a swallowed error.
    pub fn zoom_at(&mut self, screen: Point, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let world = self.screen_to_world(screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.origin.x = world.x - screen.x / self.zoom;
        self.origin.y = world.y - screen.y / self.zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4 * a.abs().max(1.0)
    }

    fn assert_point_close(a: Point, b: Point) {
        assert!(close(a.x, b.x) && close(a.y, b.y), "{a:?} vs {b:?}");
    }

    #[test]
    fn zoom_at_keeps_world_point_under_cursor_fixed() {
        let mut cam = Camera::default();
        let screen = Point::new(300.0, 220.0);
        let world_before = cam.screen_to_world(screen);
        cam.zoom_at(screen, 1.6);
        assert_point_close(cam.screen_to_world(screen), world_before);
        assert!(close(cam.zoom, 1.6));
    }

    #[test]
    fn zoom_clamps_to_bounds() {
        let mut cam = Camera::default();
        cam.zoom_at(Point::ZERO, 1e9);
        assert_eq!(cam.zoom, MAX_ZOOM);
        cam.zoom_at(Point::ZERO, 1e-9);
        assert_eq!(cam.zoom, MIN_ZOOM);
    }

    #[test]
    fn degenerate_zoom_factors_are_ignored() {
        let mut cam = Camera::default();
        cam.zoom_at(Point::new(10.0, 10.0), 0.0);
        cam.zoom_at(Point::new(10.0, 10.0), f32::NAN);
        assert_eq!(cam, Camera::default());
    }

    #[test]
    fn pan_moves_origin_against_the_delta() {
        let mut cam = Camera::default();
        cam.pan_by(Point::new(50.0, -30.0));
        assert_point_close(cam.origin, Point::new(-50.0, 30.0));
    }

    #[test]
    fn screen_world_roundtrip() {
        let cam = Camera {
            origin: Point::new(-120.5, 88.25),
            zoom: 0.4,
        };
        let world = Point::new(934.75, -12.5);
        assert_point_close(cam.screen_to_world(cam.world_to_screen(world)), world);
    }

    #[test]
    fn viewport_aabb_matches_mapping() {
        let cam = Camera::default();
        let aabb = cam.viewport_aabb(Size::new(800.0, 600.0));
        assert_point_close(aabb.min, Point::new(0.0, 0.0));
        assert_point_close(aabb.max, Point::new(800.0, 600.0));
    }

    proptest! {
        /// screen→world→screen is the identity for every clamped zoom and any
        /// finite world point (within f32 round-off at the input's magnitude).
        #[test]
        fn screen_world_roundtrip_prop(
            origin_x in -1e4_f32..1e4,
            origin_y in -1e4_f32..1e4,
            zoom in MIN_ZOOM..MAX_ZOOM,
            world_x in -1e5_f32..1e5,
            world_y in -1e5_f32..1e5,
            screen_x in 0.0_f32..4096.0,
            screen_y in 0.0_f32..4096.0,
        ) {
            let cam = Camera { origin: Point::new(origin_x, origin_y), zoom };
            let world = Point::new(world_x, world_y);
            let back = cam.world_to_screen(cam.screen_to_world(Point::new(screen_x, screen_y)));
            prop_assert!((back.x - screen_x).abs() <= 1e-2 * screen_x.abs().max(1.0));
            prop_assert!((back.y - screen_y).abs() <= 1e-2 * screen_y.abs().max(1.0));
            let home = cam.screen_to_world(cam.world_to_screen(world));
            prop_assert!((home.x - world_x).abs() <= 1e-2 * world_x.abs().max(1.0));
            prop_assert!((home.y - world_y).abs() <= 1e-2 * world_y.abs().max(1.0));
        }
    }
}
