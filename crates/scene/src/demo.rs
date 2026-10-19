//! Deterministic demo boards for interactive testing and perf smoke runs.

use crate::element::{Element, ElementId, ElementKind};
use crate::geom::{Aabb, Point, Size};

/// Multiplicative LCG (Kaneko constants); adequate for visual scatter, not for crypto.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A deterministic board of `count` rect/ellipse elements scattered over a ~4000×3000 world.
pub fn demo_board(count: u32) -> Vec<Element> {
    let mut rng = Lcg(0x00e6_7061_9173);
    let mut elements = Vec::with_capacity(count as usize);
    for i in 0..count {
        let x = rng.unit() * 3800.0;
        let y = rng.unit() * 2800.0;
        let w = 40.0 + rng.unit() * 160.0;
        let h = 40.0 + rng.unit() * 120.0;
        let r = (40.0 + rng.unit() * 200.0) as u32;
        let g = (40.0 + rng.unit() * 200.0) as u32;
        let b = (40.0 + rng.unit() * 200.0) as u32;
        let color = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        let bounds = Aabb::from_min_size(Point::new(x, y), Size::new(w, h));
        let kind = if rng.unit() < 0.5 {
            ElementKind::Rect
        } else {
            ElementKind::Ellipse
        };
        elements.push(Element::new(
            ElementId::from_raw(u64::from(i) + 1),
            kind,
            bounds,
            color,
        ));
    }
    elements
}

/// The perf smoke target: 10k mixed elements.
pub fn torture_board() -> Vec<Element> {
    demo_board(10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boards_are_deterministic_and_well_formed() {
        let a = demo_board(100);
        let b = demo_board(100);
        assert_eq!(a, b);
        assert_eq!(a.len(), 100);
        assert!(torture_board().len() == 10_000);

        let mut ids: Vec<u64> = a.iter().map(|e| e.id.raw()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 100, "ids must be unique");
        for e in &a {
            assert!(e.bounds.size().is_positive());
        }
    }
}
