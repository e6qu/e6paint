//! Layered (longest-path layering) layout, TopDown or LeftRight.
//!
//! Cycle-safe by construction: layer assignment is a monotone relaxation that converges
//! (back edges simply stop contributing once the bound is reached). Within-layer order
//! follows node declaration order, so identical graphs lay out identically.

use std::collections::HashMap;

use crate::{LayoutError, LayoutGraph, LayoutOutput, LayoutProvider};
use e6_scene::{Point, Size};

/// Axis of the layer sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Layers advance left→right; nodes stack vertically inside a layer.
    LeftRight,
    /// Layers advance top→bottom; nodes spread horizontally inside a layer.
    TopDown,
}

/// Deterministic layered layout.
#[derive(Debug, Clone)]
pub struct LayeredLayout {
    pub node_size: Size,
    pub gap: Size,
    pub direction: Direction,
}

impl LayoutProvider for LayeredLayout {
    fn layout(&self, graph: &LayoutGraph) -> Result<LayoutOutput, LayoutError> {
        crate::validate(graph)?;
        let index: HashMap<u64, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.key, i))
            .collect();

        let mut layer = vec![0usize; graph.nodes.len()];
        for _ in 0..graph.nodes.len() {
            let mut changed = false;
            for edge in &graph.edges {
                let from = index[&edge.from];
                let to = index[&edge.to];
                if layer[to] < layer[from] + 1 {
                    layer[to] = layer[from] + 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut per_layer: Vec<Vec<usize>> = Vec::new();
        for (node_index, depth) in layer.iter().enumerate() {
            let depth = *depth;
            if per_layer.len() <= depth {
                per_layer.resize(depth + 1, Vec::new());
            }
            per_layer[depth].push(node_index);
        }

        let along_is_x = self.direction == Direction::LeftRight;
        let step_along = if along_is_x {
            self.node_size.width + self.gap.width
        } else {
            self.node_size.height + self.gap.height
        };
        let step_across = if along_is_x {
            self.node_size.height + self.gap.height
        } else {
            self.node_size.width + self.gap.width
        };

        let mut positions = Vec::with_capacity(graph.nodes.len());
        for (depth, members) in per_layer.iter().enumerate() {
            let along = depth as f32 * step_along;
            let span = (members.len().saturating_sub(1) as f32) * step_across;
            for (slot, node_index) in members.iter().enumerate() {
                let across = if members.len() == 1 {
                    span * 0.5
                } else {
                    slot as f32 * step_across
                };
                let point = if along_is_x {
                    Point::new(along, across)
                } else {
                    Point::new(across, along)
                };
                positions.push((graph.nodes[*node_index].key, point));
            }
        }
        Ok(LayoutOutput { positions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LayoutGraph;

    fn graph() -> LayoutGraph {
        LayoutGraph::default()
            .node(1, Size::new(100.0, 50.0))
            .node(2, Size::new(100.0, 50.0))
            .node(3, Size::new(100.0, 50.0))
            .edge(1, 2)
            .edge(1, 3)
    }

    #[test]
    fn left_right_layers_follow_longest_path() -> Result<(), Box<dyn std::error::Error>> {
        let out = LayeredLayout {
            node_size: Size::new(100.0, 50.0),
            gap: Size::new(40.0, 30.0),
            direction: Direction::LeftRight,
        }
        .layout(&graph())?;
        let at = |key: u64| {
            out.positions
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, p)| *p)
        };
        let root = at(1).ok_or("root placed")?;
        let a = at(2).ok_or("node 2 placed")?;
        let b = at(3).ok_or("node 3 placed")?;
        assert_eq!(
            root,
            Point::new(0.0, 0.0),
            "alone in layer 0, at the origin edge"
        );
        assert_eq!(a, Point::new(140.0, 0.0));
        assert_eq!(b, Point::new(140.0, 80.0));
        Ok(())
    }

    #[test]
    fn top_down_swaps_axes() -> Result<(), Box<dyn std::error::Error>> {
        let out = LayeredLayout {
            node_size: Size::new(100.0, 50.0),
            gap: Size::new(40.0, 30.0),
            direction: Direction::TopDown,
        }
        .layout(&graph())?;
        let at = |key: u64| {
            out.positions
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, p)| *p)
        };
        let root = at(1).ok_or("root placed")?;
        let a = at(2).ok_or("node 2 placed")?;
        assert_eq!(
            root,
            Point::new(0.0, 0.0),
            "alone in layer 0, at the origin edge"
        );
        assert_eq!(a, Point::new(0.0, 80.0));
        Ok(())
    }

    #[test]
    fn cycles_terminate_with_finite_layers() -> Result<(), Box<dyn std::error::Error>> {
        let cyclic = LayoutGraph::default()
            .node(1, Size::new(10.0, 10.0))
            .node(2, Size::new(10.0, 10.0))
            .edge(1, 2)
            .edge(2, 1);
        let out = LayeredLayout {
            node_size: Size::new(10.0, 10.0),
            gap: Size::new(10.0, 10.0),
            direction: Direction::TopDown,
        }
        .layout(&cyclic)?;
        assert_eq!(
            out.positions.len(),
            2,
            "both nodes placed, relaxation converged"
        );
        Ok(())
    }
}
