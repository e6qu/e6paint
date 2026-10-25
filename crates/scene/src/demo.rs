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

/// The first-run board: a small, hand-designed welcome instead of the random
/// perf scatter. Dark-canvas palette; light text.
pub fn welcome_board() -> Vec<Element> {
    const INK: u32 = 0xFF1A_1A2E;
    const MUTED: u32 = 0xFF6F_6F85;
    const ACCENT: u32 = 0xFF4C_8DEF;

    let mut elements = Vec::new();
    let mut add = |mut element: Element| {
        element.id = ElementId::from_raw(elements.len() as u64 + 1);
        elements.push(element);
    };

    add(Element::new(
        ElementId::from_raw(1),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(40.0, 40.0), Size::new(560.0, 56.0)),
        INK,
    )
    .with_text("Welcome to e6paint".to_owned(), 40.0));

    add(Element::new(
        ElementId::from_raw(2),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(40.0, 110.0), Size::new(660.0, 28.0)),
        MUTED,
    )
    .with_text(
        "An infinite canvas for diagrams. Draw with the tool rail on the left.".to_owned(),
        18.0,
    ));

    // A frame with members: created inside, moved together.
    add(Element::new(
        ElementId::from_raw(3),
        ElementKind::Frame,
        Aabb::from_min_size(Point::new(40.0, 170.0), Size::new(340.0, 250.0)),
        ACCENT,
    )
    .with_text("Try it".to_owned(), 20.0)
    .outlined(ACCENT, 2.0));

    add(Element::new(
        ElementId::from_raw(4),
        ElementKind::Sticky,
        Aabb::from_min_size(Point::new(60.0, 220.0), Size::new(130.0, 95.0)),
        0xFFE0_6644,
    )
    .with_text("Digits 1-9 pick tools".to_owned(), 16.0));

    add(Element::new(
        ElementId::from_raw(5),
        ElementKind::Rect,
        Aabb::from_min_size(Point::new(220.0, 230.0), Size::new(120.0, 80.0)),
        0xFFFF_FFFF,
    )
    .with_text("Drag me".to_owned(), 16.0)
    .outlined(0xFF8A_8AA8, 2.0));

    // Connector: rect -> ellipse, demonstrating arrows.
    add(Element::new(
        ElementId::from_raw(6),
        ElementKind::Connector,
        Aabb::from_min_size(Point::new(340.0, 265.0), Size::new(70.0, 20.0)),
        MUTED,
    )
    .with_points(vec![Point::new(345.0, 270.0), Point::new(405.0, 280.0)]));

    add(Element::new(
        ElementId::from_raw(7),
        ElementKind::Ellipse,
        Aabb::from_min_size(Point::new(408.0, 255.0), Size::new(90.0, 65.0)),
        0xFF2E_5E4E,
    )
    .with_text("fun".to_owned(), 16.0)
    .outlined(MUTED, 2.0));

    add(Element::new(
        ElementId::from_raw(8),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(440.0, 170.0), Size::new(420.0, 28.0)),
        INK,
    )
    .with_text(
        "Multiplayer: top bar -> Share -> open the link on another device.".to_owned(),
        18.0,
    ));

    add(Element::new(
        ElementId::from_raw(9),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(440.0, 210.0), Size::new(420.0, 28.0)),
        MUTED,
    )
    .with_text(
        "Diagrams from code: the mol tab in the drawer, cmd/ctrl+Enter renders.".to_owned(),
        18.0,
    ));

    add(Element::new(
        ElementId::from_raw(10),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(440.0, 250.0), Size::new(420.0, 28.0)),
        MUTED,
    )
    .with_text(
        "Export: cmd/ctrl+E for SVG (+shift for PNG).".to_owned(),
        18.0,
    ));

    add(Element::new(
        ElementId::from_raw(11),
        ElementKind::Text,
        Aabb::from_min_size(Point::new(440.0, 290.0), Size::new(420.0, 28.0)),
        MUTED,
    )
    .with_text("Everything autosaves in this browser.".to_owned(), 18.0));

    let _ = add; // closure used above
    elements
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
