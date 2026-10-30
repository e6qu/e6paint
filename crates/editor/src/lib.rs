//! Tools, selection, and the pointer-gesture reducer.
//!
//! [`gesture`] is the pure interaction state machine: pointer events in,
//! board commands out. The shell adapts egui input to events and applies the
//! commands to the document.

pub mod align;
pub mod gesture;
pub mod resize;

use std::collections::BTreeSet;

use e6_scene::{Aabb, ElementId};

/// Board tools (locked set for v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Pan,
    Pen,
    Rect,
    Ellipse,
    Connector,
    Text,
    Sticky,
    Image,
    Frame,
}

/// The current selection: a set of element ids with stable ordering.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    ids: BTreeSet<ElementId>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `id` if absent, remove it if present.
    pub fn toggle(&mut self, id: ElementId) {
        if !self.ids.remove(&id) {
            self.ids.insert(id);
        }
    }

    pub fn set(&mut self, ids: impl IntoIterator<Item = ElementId>) {
        self.ids.clear();
        self.ids.extend(ids);
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.ids.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: ElementId) -> bool {
        self.ids.contains(&id)
    }

    /// Union of member bounds, resolved through `bounds_of`. `None` while empty or when any
    /// member fails to resolve (a stale selection is surfaced, not silently shrunk).
    pub fn bounds(&self, bounds_of: impl Fn(ElementId) -> Option<Aabb>) -> Option<Aabb> {
        let mut acc: Option<Aabb> = None;
        for id in self.ids.iter().copied() {
            let b = bounds_of(id)?;
            acc = Some(match acc {
                Some(a) => a.union(b),
                None => b,
            });
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use e6_scene::{Point, Size};

    fn id(raw: u64) -> ElementId {
        ElementId::from_raw(raw)
    }

    fn resolve(id: ElementId) -> Option<Aabb> {
        match id.raw() {
            1 => Some(Aabb::from_min_size(
                Point::new(0.0, 0.0),
                Size::new(10.0, 10.0),
            )),
            2 => Some(Aabb::from_min_size(
                Point::new(20.0, 5.0),
                Size::new(10.0, 10.0),
            )),
            _ => None,
        }
    }

    #[test]
    fn toggle_and_clear() {
        let mut sel = Selection::new();
        assert!(sel.is_empty());
        sel.toggle(id(1));
        sel.toggle(id(2));
        assert_eq!(sel.len(), 2);
        sel.toggle(id(2));
        assert_eq!(sel.iter().collect::<Vec<_>>(), vec![id(1)]);
        sel.clear();
        assert!(sel.is_empty());
    }

    #[test]
    fn bounds_unions_members_and_surfaces_stale_ids() {
        let mut sel = Selection::new();
        sel.set([id(1), id(2)]);
        let b = sel.bounds(resolve);
        assert_eq!(
            b,
            Some(Aabb::from_min_size(
                Point::new(0.0, 0.0),
                Size::new(30.0, 15.0)
            ))
        );

        sel.set([id(1), id(99)]);
        assert_eq!(
            sel.bounds(resolve),
            None,
            "stale selection must not silently shrink"
        );
    }
}
