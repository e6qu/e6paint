//! Spatial index over element bounds.
//!
//! The linear [`NaiveIndex`] serves the first builds; the quadtree replaces it behind
//! the same port when LOD lands, so call sites never change.

use crate::element::ElementId;
use crate::geom::Aabb;

/// Window queries over element bounds.
pub trait SpatialIndex {
    fn insert(&mut self, id: ElementId, bounds: Aabb);

    fn remove(&mut self, id: ElementId);

    /// IDs whose bounds intersect `region`. Order is unspecified.
    fn query(&self, region: Aabb) -> Vec<ElementId>;
}

/// O(n) scan over all bounds. Correct and cache-friendly; fine at walking-skeleton scale.
#[derive(Debug, Default, Clone)]
pub struct NaiveIndex {
    entries: Vec<(ElementId, Aabb)>,
}

impl NaiveIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SpatialIndex for NaiveIndex {
    fn insert(&mut self, id: ElementId, bounds: Aabb) {
        self.entries.push((id, bounds));
    }

    fn remove(&mut self, id: ElementId) {
        self.entries.retain(|(existing, _)| *existing != id);
    }

    fn query(&self, region: Aabb) -> Vec<ElementId> {
        self.entries
            .iter()
            .filter(|(_, b)| b.intersects(region))
            .map(|(id, _)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, Size};

    fn box_at(x: f32, y: f32) -> Aabb {
        Aabb::from_min_size(Point::new(x, y), Size::new(10.0, 10.0))
    }

    fn id(raw: u64) -> ElementId {
        ElementId::from_raw(raw)
    }

    #[test]
    fn query_returns_exactly_the_intersecting_ids() {
        let mut index = NaiveIndex::new();
        index.insert(id(1), box_at(0.0, 0.0));
        index.insert(id(2), box_at(50.0, 50.0));
        index.insert(id(3), box_at(5.0, 5.0));

        let mut hits = index.query(box_at(0.0, 0.0));
        hits.sort();
        assert_eq!(hits, vec![id(1), id(3)]);
    }

    #[test]
    fn remove_drops_only_the_named_element() {
        let mut index = NaiveIndex::new();
        index.insert(id(1), box_at(0.0, 0.0));
        index.insert(id(2), box_at(1.0, 1.0));
        index.remove(id(1));

        let hits = index.query(box_at(0.0, 0.0));
        assert_eq!(hits, vec![id(2)]);
    }
}
