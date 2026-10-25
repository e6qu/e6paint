//! Handle-based resizing for a single selected element (pure math).
//!
//! Four corner handles; dragging a corner moves that corner of the bounds.
//! Crossing edges normalize (flip), with a minimum size floor.

use e6_scene::{Aabb, Point};

/// The four corner handles of a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

/// The corner handle under `world`, if the point is inside its grab square.
/// Grab squares are the four corner quadrants of the bounds, shrunk to `grab`
/// world units around the exact corner.
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
    if near_x_min && near_y_min {
        Some(ResizeHandle::NorthWest)
    } else if near_x_max && near_y_min {
        Some(ResizeHandle::NorthEast)
    } else if near_x_min && near_y_max {
        Some(ResizeHandle::SouthWest)
    } else if near_x_max && near_y_max {
        Some(ResizeHandle::SouthEast)
    } else {
        None
    }
}

/// Minimum accepted size (world units).
pub const MIN_SIZE: f32 = 8.0;

/// Resize `original` by moving `handle` to `world`. The opposite corner stays
/// anchored. Edges may cross; the result is normalized. The size never shrinks
/// below [`MIN_SIZE`].
pub fn resize_bounds(original: Aabb, handle: ResizeHandle, world: Point) -> Aabb {
    let mut min = original.min;
    let mut max = original.max;
    match handle {
        ResizeHandle::NorthWest => {
            min = Point::new(world.x, world.y);
        }
        ResizeHandle::NorthEast => {
            min.y = world.y;
            max.x = world.x;
        }
        ResizeHandle::SouthWest => {
            min.x = world.x;
            max.y = world.y;
        }
        ResizeHandle::SouthEast => {
            max = Point::new(world.x, world.y);
        }
    }
    // Enforce the minimum size without flipping: clamp the dragged corner
    // against the anchored one.
    if max.x - min.x < MIN_SIZE {
        if min.x != original.min.x && original.max.x - original.min.x >= MIN_SIZE {
            min.x = max.x - MIN_SIZE;
        } else {
            max.x = min.x + MIN_SIZE;
        }
    }
    if max.y - min.y < MIN_SIZE {
        if min.y != original.min.y && original.max.y - original.min.y >= MIN_SIZE {
            min.y = max.y - MIN_SIZE;
        } else {
            max.y = min.y + MIN_SIZE;
        }
    }
    if min.x > max.x {
        std::mem::swap(&mut min.x, &mut max.x);
    }
    if min.y > max.y {
        std::mem::swap(&mut min.y, &mut max.y);
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
    fn crossing_edges_normalize() {
        let original = square(0.0, 0.0, 100.0);
        // Drag SE far past the NW corner: the rect flips and stays valid.
        let flipped = resize_bounds(original, ResizeHandle::SouthEast, Point::new(-80.0, -60.0));
        assert!(flipped.min.x <= flipped.max.x);
        assert!(flipped.min.y <= flipped.max.y);
        assert!(flipped.size().width >= MIN_SIZE);
        assert!(flipped.size().height >= MIN_SIZE);
    }

    #[test]
    fn min_size_is_enforced() {
        let original = square(0.0, 0.0, 100.0);
        let tiny = resize_bounds(original, ResizeHandle::NorthWest, Point::new(99.0, 99.0));
        assert!(tiny.size().width >= MIN_SIZE);
        assert!(tiny.size().height >= MIN_SIZE);
    }

    #[test]
    fn handle_at_hits_only_corners() {
        let bounds = square(0.0, 0.0, 100.0);
        assert_eq!(
            handle_at(bounds, Point::new(0.0, 0.0), 10.0),
            Some(ResizeHandle::NorthWest)
        );
        assert_eq!(
            handle_at(bounds, Point::new(100.0, 100.0), 10.0),
            Some(ResizeHandle::SouthEast)
        );
        assert_eq!(handle_at(bounds, Point::new(50.0, 50.0), 10.0), None);
        assert_eq!(handle_at(bounds, Point::new(0.0, 50.0), 10.0), None);
    }
}
