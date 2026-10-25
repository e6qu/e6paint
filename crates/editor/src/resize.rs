//! Handle-based resizing for a single selected element (pure math).
//!
//! Eight handles: four corners and four edge midpoints. Dragging a handle
//! moves that side/corner of the bounds; the opposite side stays anchored.
//! The size is clamped to a minimum — dragging past the anchor clamps at the
//! floor instead of flipping (Miro behavior).

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
}
