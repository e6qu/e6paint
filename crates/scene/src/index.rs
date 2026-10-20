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
    use proptest::prelude::*;

    fn box_at(x: f32, y: f32) -> Aabb {
        Aabb::from_min_size(Point::new(x, y), Size::new(10.0, 10.0))
    }

    fn id(raw: u64) -> ElementId {
        ElementId::from_raw(raw)
    }

    proptest! {
        /// Containment soundness: a query over a stored rect always returns its
        /// id, and a removed id never returns. Raw ids derive from the position
        /// index, so they are unique by construction.
        #[test]
        fn query_is_sound_for_arbitrary_rects(
            positions in proptest::collection::vec(
                (-50.0_f64..450.0, -50.0_f64..450.0),
                1..25,
            ),
            probe_index in 0usize..25,
            remove_probe in proptest::bool::ANY,
        ) {
            let mut index = NaiveIndex::new();
            let mut specs: Vec<(ElementId, Aabb)> = Vec::new();
            for (position_index, (x, y)) in positions.iter().enumerate() {
                specs.push((
                    ElementId::from_raw(position_index as u64),
                    Aabb::from_min_size(
                        Point::new(*x as f32, *y as f32),
                        Size::new(40.0, 40.0),
                    ),
                ));
            }
            for (element_id, bounds) in &specs {
                index.insert(*element_id, *bounds);
            }

            let probe = &specs[probe_index % specs.len()];
            let probe_id = probe.0;
            let center = probe.1.center();
            let query_region = Aabb::from_min_size(
                Point::new(center.x - 1.0, center.y - 1.0),
                Size::new(2.0, 2.0),
            );
            prop_assert!(index.query(query_region).contains(&probe_id));

            if remove_probe {
                index.remove(probe_id);
                prop_assert!(!index.query(query_region).contains(&probe_id));
            } else {
                // Removing everything else must not disturb the probe.
                for (element_id, _) in &specs {
                    if *element_id != probe_id {
                        index.remove(*element_id);
                    }
                }
                prop_assert!(index.query(query_region).contains(&probe_id));
            }
        }
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
