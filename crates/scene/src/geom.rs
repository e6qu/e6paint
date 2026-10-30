//! Points, sizes, and axis-aligned boxes in board space.

/// A point or offset in 2D space (world or screen, depending on context).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A width/height pair.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn is_positive(self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// An axis-aligned bounding box, stored as corner points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Point,
    pub max: Point,
}

impl Aabb {
    /// Box from its minimum corner and a size. The caller must pass a positive size —
    /// use [`Aabb::checked_from_min_size`] for untrusted input.
    pub fn from_min_size(min: Point, size: Size) -> Self {
        debug_assert!(size.is_positive(), "degenerate bounds are meaningless");
        Self {
            min,
            max: Point::new(min.x + size.width, min.y + size.height),
        }
    }

    /// [`Aabb::from_min_size`] for untrusted input: `None` unless the size is positive.
    pub fn checked_from_min_size(min: Point, size: Size) -> Option<Self> {
        if size.is_positive() {
            Some(Self::from_min_size(min, size))
        } else {
            None
        }
    }

    /// Bounds exactly containing `points`. `None` when empty; a single point yields a
    /// zero-size box (callers decide whether that is meaningful for their kind).
    pub fn from_points(points: &[Point]) -> Option<Self> {
        let (first, rest) = points.split_first()?;
        let mut bounds = Self {
            min: *first,
            max: Point::new(first.x, first.y),
        };
        for p in rest {
            bounds = bounds.union(Self {
                min: *p,
                max: Point::new(p.x, p.y),
            });
        }
        Some(bounds)
    }

    pub fn size(self) -> Size {
        Size::new(self.max.x - self.min.x, self.max.y - self.min.y)
    }

    pub fn center(self) -> Point {
        Point::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    /// The axis-aligned box covering both points.
    pub fn from_two_points(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }
}

/// Rotate an offset vector by `rotation` radians, counter-clockwise about the
/// origin (the same convention as the renderer's `Affine::rotate`).
pub fn rotate_vec(dx: f32, dy: f32, rotation: f32) -> (f32, f32) {
    let (sin, cos) = f64::from(rotation).sin_cos();
    let (dx, dy) = (f64::from(dx), f64::from(dy));
    ((dx * cos - dy * sin) as f32, (dx * sin + dy * cos) as f32)
}

/// The inverse of [`rotate_vec`].
pub fn unrotate_vec(dx: f32, dy: f32, rotation: f32) -> (f32, f32) {
    rotate_vec(dx, dy, -rotation)
}

/// Rotate `p` about `center` by `rotation` radians.
pub fn rotate_about(p: Point, center: Point, rotation: f32) -> Point {
    if rotation == 0.0 {
        return p;
    }
    let (rx, ry) = rotate_vec(p.x - center.x, p.y - center.y, rotation);
    Point::new(rx + center.x, ry + center.y)
}

/// The inverse of [`rotate_about`].
pub fn unrotate_about(p: Point, center: Point, rotation: f32) -> Point {
    rotate_about(p, center, -rotation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aabb(x: f32, y: f32, w: f32, h: f32) -> Aabb {
        Aabb::from_min_size(Point::new(x, y), Size::new(w, h))
    }

    #[test]
    fn checked_constructor_rejects_degenerate_sizes() {
        assert!(Aabb::checked_from_min_size(Point::ZERO, Size::new(10.0, 0.0)).is_none());
        assert!(Aabb::checked_from_min_size(Point::ZERO, Size::new(-1.0, 4.0)).is_none());
        assert!(Aabb::checked_from_min_size(Point::new(1.0, 2.0), Size::new(3.0, 4.0)).is_some());
    }

    #[test]
    fn geometry_primitives() {
        let a = aabb(0.0, 0.0, 10.0, 10.0);
        assert_eq!(a.size(), Size::new(10.0, 10.0));
        assert_eq!(a.center(), Point::new(5.0, 5.0));
        assert!(a.contains(Point::new(5.0, 5.0)));
        assert!(!a.contains(Point::new(11.0, 5.0)));
    }

    #[test]
    fn intersection_and_union() {
        let a = aabb(0.0, 0.0, 10.0, 10.0);
        let b = aabb(5.0, 5.0, 10.0, 10.0);
        let c = aabb(20.0, 20.0, 1.0, 1.0);
        assert!(a.intersects(b) && b.intersects(a));
        assert!(!a.intersects(c));
        assert_eq!(a.union(c), aabb(0.0, 0.0, 21.0, 21.0));
    }

    #[test]
    fn bounds_of_points() {
        assert!(Aabb::from_points(&[]).is_none());
        let span = Aabb::from_points(&[Point::new(3.0, -2.0), Point::new(-1.0, 5.0)]);
        assert_eq!(span, Some(aabb(-1.0, -2.0, 4.0, 7.0)));
    }
}
