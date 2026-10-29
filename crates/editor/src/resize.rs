//! Handle-based resizing for a single selected element (pure math).
//!
//! Eight handles: four corners and four edge midpoints. Dragging a handle
//! moves that side/corner of the bounds; the opposite side stays anchored.
//! The size is clamped to a minimum — dragging past the anchor clamps at the
//! floor instead of flipping (Miro behavior).
//!
//! Rotated elements resize in their local frame: the pointer offset from the
//! fixed anchor is un-rotated into element space, and the stored bounds are
//! re-anchored so the painted anchor stays put while the painted handle
//! follows the pointer.

use e6_scene::{Aabb, Point};

/// The eight resize handles of a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

/// Minimum accepted size (world units).
pub const MIN_SIZE: f32 = 8.0;

/// The handle under `world`, if any. Corners win over edges; grab zones extend
/// `grab` world units around each handle point (and along the edges).
pub fn handle_at(bounds: Aabb, world: Point, grab: f32) -> Option<ResizeHandle> {
    let size = bounds.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return None;
    }
    let near = |value: f32, target: f32| (value - target).abs() <= grab;
    let near_x_min = near(world.x, bounds.min.x);
    let near_x_max = near(world.x, bounds.max.x);
    let near_y_min = near(world.y, bounds.min.y);
    let near_y_max = near(world.y, bounds.max.y);

    // Corners first.
    if near_x_min && near_y_min {
        return Some(ResizeHandle::NorthWest);
    }
    if near_x_max && near_y_min {
        return Some(ResizeHandle::NorthEast);
    }
    if near_x_min && near_y_max {
        return Some(ResizeHandle::SouthWest);
    }
    if near_x_max && near_y_max {
        return Some(ResizeHandle::SouthEast);
    }
    // Edges: near the edge coordinate and within the span (padded by grab).
    let in_y_span = world.y >= bounds.min.y - grab && world.y <= bounds.max.y + grab;
    let in_x_span = world.x >= bounds.min.x - grab && world.x <= bounds.max.x + grab;
    if near_x_min && in_y_span {
        return Some(ResizeHandle::West);
    }
    if near_x_max && in_y_span {
        return Some(ResizeHandle::East);
    }
    if near_y_min && in_x_span {
        return Some(ResizeHandle::North);
    }
    if near_y_max && in_x_span {
        return Some(ResizeHandle::South);
    }
    None
}

/// Resize `original` by moving `handle` to `world`. The opposite side stays
/// anchored; the size clamps at [`MIN_SIZE`] instead of flipping.
pub fn resize_bounds(original: Aabb, handle: ResizeHandle, world: Point) -> Aabb {
    let mut min = original.min;
    let mut max = original.max;
    let moves_min_x = matches!(
        handle,
        ResizeHandle::NorthWest | ResizeHandle::SouthWest | ResizeHandle::West
    );
    let moves_max_x = matches!(
        handle,
        ResizeHandle::NorthEast | ResizeHandle::SouthEast | ResizeHandle::East
    );
    let moves_min_y = matches!(
        handle,
        ResizeHandle::NorthWest | ResizeHandle::NorthEast | ResizeHandle::North
    );
    let moves_max_y = matches!(
        handle,
        ResizeHandle::SouthWest | ResizeHandle::SouthEast | ResizeHandle::South
    );

    if moves_min_x {
        min.x = world.x.min(max.x - MIN_SIZE);
    }
    if moves_max_x {
        max.x = world.x.max(min.x + MIN_SIZE);
    }
    if moves_min_y {
        min.y = world.y.min(max.y - MIN_SIZE);
    }
    if moves_max_y {
        max.y = world.y.max(min.y + MIN_SIZE);
    }
    Aabb::from_min_size(min, e6_scene::Size::new(max.x - min.x, max.y - min.y))
}

/// The local-frame point the `handle` sits on (bounds corners for corners,
/// edge midpoints for edges).
pub fn handle_point(handle: ResizeHandle, bounds: Aabb) -> Point {
    let center = bounds.center();
    match handle {
        ResizeHandle::NorthWest => Point::new(bounds.min.x, bounds.min.y),
        ResizeHandle::North => Point::new(center.x, bounds.min.y),
        ResizeHandle::NorthEast => Point::new(bounds.max.x, bounds.min.y),
        ResizeHandle::East => Point::new(bounds.max.x, center.y),
        ResizeHandle::SouthEast => Point::new(bounds.max.x, bounds.max.y),
        ResizeHandle::South => Point::new(center.x, bounds.max.y),
        ResizeHandle::SouthWest => Point::new(bounds.min.x, bounds.max.y),
        ResizeHandle::West => Point::new(bounds.min.x, center.y),
    }
}

/// The local-frame point that stays fixed while `handle` drags: the opposite
/// corner for corner handles, the opposite edge's midpoint for edges.
pub fn anchor_point(handle: ResizeHandle, bounds: Aabb) -> Point {
    handle_point(opposite(handle), bounds)
}

/// The handle diagonally / directly opposite `handle`.
pub fn opposite(handle: ResizeHandle) -> ResizeHandle {
    match handle {
        ResizeHandle::NorthWest => ResizeHandle::SouthEast,
        ResizeHandle::North => ResizeHandle::South,
        ResizeHandle::NorthEast => ResizeHandle::SouthWest,
        ResizeHandle::East => ResizeHandle::West,
        ResizeHandle::SouthEast => ResizeHandle::NorthWest,
        ResizeHandle::South => ResizeHandle::North,
        ResizeHandle::SouthWest => ResizeHandle::NorthEast,
        ResizeHandle::West => ResizeHandle::East,
    }
}

/// Resize a rotated element by moving `handle` to `pointer` (world space).
///
/// The resize happens in the element's local frame; the returned bounds are
/// re-anchored so the painted anchor point stays exactly at `anchor_world`
/// and the painted handle corner follows the pointer (clamping at
/// [`MIN_SIZE`] as in the axis-aligned path). `anchor_world` is the painted
/// position of [`anchor_point`] for the gesture's starting bounds.
pub fn resize_bounds_rotated(
    original: Aabb,
    handle: ResizeHandle,
    rotation: f32,
    pointer: Point,
    anchor_world: Point,
) -> Aabb {
    // The pointer offset from the fixed anchor, in the element's local frame.
    // Since the painted map is an affine image of the local bounds, the new
    // local size is fully determined by this offset.
    let (u, v) = e6_scene::geom::unrotate_vec(
        pointer.x - anchor_world.x,
        pointer.y - anchor_world.y,
        rotation,
    );
    let width = original.size().width;
    let height = original.size().height;
    let (new_w, new_h) = match handle {
        ResizeHandle::SouthEast => (u.max(MIN_SIZE), v.max(MIN_SIZE)),
        ResizeHandle::NorthEast => (u.max(MIN_SIZE), (-v).max(MIN_SIZE)),
        ResizeHandle::SouthWest => ((-u).max(MIN_SIZE), v.max(MIN_SIZE)),
        ResizeHandle::NorthWest => ((-u).max(MIN_SIZE), (-v).max(MIN_SIZE)),
        ResizeHandle::East => (u.max(MIN_SIZE), height),
        ResizeHandle::West => ((-u).max(MIN_SIZE), height),
        ResizeHandle::South => (width, v.max(MIN_SIZE)),
        ResizeHandle::North => (width, (-v).max(MIN_SIZE)),
    };
    // Rebuild the bounds from the fixed anchor sides.
    let moves_min_x = matches!(
        handle,
        ResizeHandle::NorthWest | ResizeHandle::SouthWest | ResizeHandle::West
    );
    let moves_min_y = matches!(
        handle,
        ResizeHandle::NorthWest | ResizeHandle::NorthEast | ResizeHandle::North
    );
    let min_x = if moves_min_x {
        original.max.x - new_w
    } else {
        original.min.x
    };
    let min_y = if moves_min_y {
        original.max.y - new_h
    } else {
        original.min.y
    };
    let next = Aabb::from_min_size(Point::new(min_x, min_y), e6_scene::Size::new(new_w, new_h));
    // Re-anchor: translation commutes with the rotation about the center, so
    // shifting the stored bounds shifts the painted shape identically.
    let painted_anchor =
        e6_scene::geom::rotate_about(anchor_point(handle, next), next.center(), rotation);
    let delta = Point::new(
        anchor_world.x - painted_anchor.x,
        anchor_world.y - painted_anchor.y,
    );
    Aabb::from_min_size(
        Point::new(next.min.x + delta.x, next.min.y + delta.y),
        next.size(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, side: f32) -> Aabb {
        Aabb::from_min_size(Point::new(x, y), e6_scene::Size::new(side, side))
    }

    #[test]
    fn south_east_drag_grows_and_shrinks() {
        let original = square(0.0, 0.0, 100.0);
        let grown = resize_bounds(original, ResizeHandle::SouthEast, Point::new(150.0, 140.0));
        assert_eq!(grown.min, Point::new(0.0, 0.0));
        assert_eq!(grown.max, Point::new(150.0, 140.0));

        let shrunk = resize_bounds(original, ResizeHandle::SouthEast, Point::new(40.0, 30.0));
        assert_eq!(shrunk.max, Point::new(40.0, 30.0));
        assert!(shrunk.size().width >= MIN_SIZE && shrunk.size().height >= MIN_SIZE);
    }

    #[test]
    fn north_west_drag_moves_the_min() {
        let original = square(100.0, 100.0, 100.0);
        let resized = resize_bounds(original, ResizeHandle::NorthWest, Point::new(60.0, 60.0));
        assert_eq!(resized.min, Point::new(60.0, 60.0));
        assert_eq!(resized.max, Point::new(200.0, 200.0));
    }

    #[test]
    fn edge_handles_resize_one_dimension_only() {
        let original = square(0.0, 0.0, 100.0);
        // East: width changes, height untouched.
        let east = resize_bounds(original, ResizeHandle::East, Point::new(180.0, 999.0));
        assert_eq!(east.max, Point::new(180.0, 100.0));
        assert_eq!(east.min, Point::new(0.0, 0.0));
        // South: height changes, width untouched.
        let south = resize_bounds(original, ResizeHandle::South, Point::new(999.0, 160.0));
        assert_eq!(south.max, Point::new(100.0, 160.0));
        assert_eq!(south.min, Point::new(0.0, 0.0));
    }

    #[test]
    fn dragging_outward_grows_dragging_inward_clamps() {
        let original = square(0.0, 0.0, 100.0);
        // NW dragged outward: the rect grows; the anchor stays put.
        let grown = resize_bounds(original, ResizeHandle::NorthWest, Point::new(-80.0, -60.0));
        assert_eq!(grown.min, Point::new(-80.0, -60.0));
        assert_eq!(grown.max, Point::new(100.0, 100.0));
        // NW dragged inward past the anchor: clamps at the min size.
        let clamped = resize_bounds(original, ResizeHandle::NorthWest, Point::new(99.0, 99.0));
        assert_eq!(clamped.min, Point::new(92.0, 92.0));
        assert_eq!(clamped.max, Point::new(100.0, 100.0));
    }

    #[test]
    fn handle_at_hits_corners_edges_and_nothing_else() {
        let bounds = square(0.0, 0.0, 100.0);
        assert_eq!(
            handle_at(bounds, Point::new(0.0, 0.0), 10.0),
            Some(ResizeHandle::NorthWest)
        );
        assert_eq!(
            handle_at(bounds, Point::new(100.0, 100.0), 10.0),
            Some(ResizeHandle::SouthEast)
        );
        assert_eq!(
            handle_at(bounds, Point::new(100.0, 50.0), 10.0),
            Some(ResizeHandle::East)
        );
        assert_eq!(
            handle_at(bounds, Point::new(50.0, 0.0), 10.0),
            Some(ResizeHandle::North)
        );
        // Center: no handle.
        assert_eq!(handle_at(bounds, Point::new(50.0, 50.0), 10.0), None);
        // Beyond the span: no edge handle.
        assert_eq!(handle_at(bounds, Point::new(100.0, 500.0), 10.0), None);
    }

    /// Painted position of a local point for an element with `bounds` and
    /// `rotation` — the same affine map the renderer applies.
    fn painted(bounds: Aabb, rotation: f32, local: Point) -> Point {
        e6_scene::geom::rotate_about(local, bounds.center(), rotation)
    }

    fn near(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn opposite_is_involutive_and_pairwise() {
        for handle in [
            ResizeHandle::NorthWest,
            ResizeHandle::North,
            ResizeHandle::NorthEast,
            ResizeHandle::East,
            ResizeHandle::SouthEast,
            ResizeHandle::South,
            ResizeHandle::SouthWest,
            ResizeHandle::West,
        ] {
            assert_eq!(opposite(opposite(handle)), handle);
        }
        assert_eq!(opposite(ResizeHandle::NorthWest), ResizeHandle::SouthEast);
        assert_eq!(opposite(ResizeHandle::East), ResizeHandle::West);
    }

    #[test]
    fn quarter_turn_corner_drag_tracks_the_pointer_and_fixes_the_anchor() {
        use core::f32::consts::FRAC_PI_2;
        // 100x50 element at the origin, rotated a quarter turn: it paints as a
        // 50x100 box spanning x in [25, 75], y in [-25, 75].
        let original = square(0.0, 0.0, 100.0);
        let original = Aabb::from_min_size(
            original.min,
            e6_scene::Size::new(original.size().width, original.size().height * 0.5),
        );
        let rotation = FRAC_PI_2;
        let handle = ResizeHandle::NorthEast;
        let anchor_world = painted(original, rotation, anchor_point(handle, original));
        assert!(near(anchor_world, Point::new(25.0, -25.0)));

        // Drag the painted NE corner (75, 75) to (55, 100).
        let pointer = Point::new(55.0, 100.0);
        let next = resize_bounds_rotated(original, handle, rotation, pointer, anchor_world);

        // The painted anchor did not move.
        assert!(near(
            painted(next, rotation, anchor_point(handle, next)),
            anchor_world
        ));
        // The painted dragged corner landed on the pointer.
        assert!(near(
            painted(next, rotation, handle_point(handle, next)),
            pointer
        ));
        // The local size grew to 125x30 (the drag in the element's frame).
        assert!((next.size().width - 125.0).abs() < 1e-3);
        assert!((next.size().height - 30.0).abs() < 1e-3);
    }

    #[test]
    fn quarter_turn_edge_drag_tracks_the_pointer_perpendicular_to_the_edge() {
        use core::f32::consts::FRAC_PI_2;
        let original = Aabb::from_min_size(Point::new(0.0, 0.0), e6_scene::Size::new(100.0, 50.0));
        let rotation = FRAC_PI_2;
        let handle = ResizeHandle::East;
        let anchor_world = painted(original, rotation, anchor_point(handle, original));
        // The quarter turn paints the local East edge horizontally through the
        // handle at (50, 75); width responds to vertical pointer motion.
        let pointer = Point::new(50.0, 115.0);
        let next = resize_bounds_rotated(original, handle, rotation, pointer, anchor_world);
        assert!(near(
            painted(next, rotation, anchor_point(handle, next)),
            anchor_world
        ));
        assert!(near(
            painted(next, rotation, handle_point(handle, next)),
            pointer
        ));
        // Only the width changed; the pointer's along-edge motion is ignored.
        assert!((next.size().width - 140.0).abs() < 1e-3);
        assert!((next.size().height - 50.0).abs() < 1e-3);
    }

    #[test]
    fn rotated_drag_past_the_anchor_clamps_at_min_size() {
        use core::f32::consts::FRAC_PI_2;
        let original = Aabb::from_min_size(Point::new(0.0, 0.0), e6_scene::Size::new(100.0, 50.0));
        let rotation = FRAC_PI_2;
        let handle = ResizeHandle::NorthEast;
        let anchor_world = painted(original, rotation, anchor_point(handle, original));
        // Pointer pulled hard past the anchor: both dimensions clamp.
        let pointer = Point::new(30.0, 0.0);
        let next = resize_bounds_rotated(original, handle, rotation, pointer, anchor_world);
        assert!(next.size().width >= MIN_SIZE && next.size().height >= MIN_SIZE);
        assert!(near(
            painted(next, rotation, anchor_point(handle, next)),
            anchor_world
        ));
    }

    #[test]
    fn rotated_and_axis_aligned_agree_at_zero_rotation() {
        let original = square(10.0, 20.0, 100.0);
        let handle = ResizeHandle::SouthEast;
        let pointer = Point::new(160.0, 150.0);
        let anchor_world = anchor_point(handle, original);
        let rotated = resize_bounds_rotated(original, handle, 0.0, pointer, anchor_world);
        assert_eq!(rotated, resize_bounds(original, handle, pointer));
    }
}
