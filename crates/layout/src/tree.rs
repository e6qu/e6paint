//! Tidy tree layout: parents center over their children, leaves flow left→right.
//!
//! Roots (nodes without incoming edges) are laid out side by side in declaration order.
//! Back edges (cycles) are ignored during placement.

use std::collections::{HashMap, HashSet};

use crate::{LayoutError, LayoutGraph, LayoutOutput, LayoutProvider};
use e6_scene::{Point, Size};

/// Deterministic tree layout for parent→child graphs.
#[derive(Debug, Clone)]
pub struct TreeLayout {
    pub node_size: Size,
    pub gap: Size,
}

impl LayoutProvider for TreeLayout {
    fn layout(&self, graph: &LayoutGraph) -> Result<LayoutOutput, LayoutError> {
        crate::validate(graph)?;
        let index: HashMap<u64, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.key, i))
            .collect();
        let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut has_parent: HashSet<u64> = HashSet::new();
        for edge in &graph.edges {
            // A tree layout duplicates multi-parent nodes (each parent would lay
            // the child out again); rejecting is the loud contract.
            if !has_parent.insert(edge.to) {
                return Err(LayoutError::MultipleParents { key: edge.to });
            }
            children.entry(edge.from).or_default().push(edge.to);
        }

        let step_x = self.node_size.width + self.gap.width;
        let step_y = self.node_size.height + self.gap.height;
        let mut positions: Vec<(u64, Point)> = Vec::with_capacity(graph.nodes.len());
        let mut placed: HashSet<u64> = HashSet::new();
        let mut cursor_x = 0.0_f32;

        for node in &graph.nodes {
            if has_parent.contains(&node.key) || !placed.insert(node.key) {
                continue;
            }
            place_root(
                &index,
                &children,
                &mut placed,
                node.key,
                0,
                step_x,
                step_y,
                &mut cursor_x,
                &mut positions,
            );
        }
        // Nodes only reachable through cycles still deserve a place.
        for node in &graph.nodes {
            if placed.insert(node.key) {
                positions.push((node.key, Point::new(cursor_x, 0.0)));
                cursor_x += step_x;
            }
        }
        Ok(LayoutOutput { positions })
    }
}

#[allow(clippy::too_many_arguments)]
fn place_root(
    index: &HashMap<u64, usize>,
    children: &HashMap<u64, Vec<u64>>,
    placed: &mut HashSet<u64>,
    key: u64,
    depth: usize,
    step_x: f32,
    step_y: f32,
    cursor_x: &mut f32,
    positions: &mut Vec<(u64, Point)>,
) {
    let kids: Vec<u64> = children
        .get(&key)
        .map(|kids| {
            kids.iter()
                .copied()
                .filter(|kid| index.contains_key(kid) && !placed.contains(kid))
                .collect()
        })
        .unwrap_or_default();

    if kids.is_empty() {
        positions.push((key, Point::new(*cursor_x, depth as f32 * step_y)));
        *cursor_x += step_x;
        placed.insert(key);
        return;
    }
    let child_depth = depth + 1;
    let start = *cursor_x;
    for kid in &kids {
        place_root(
            index,
            children,
            placed,
            *kid,
            child_depth,
            step_x,
            step_y,
            cursor_x,
            positions,
        );
    }
    let first = positions
        .iter()
        .find(|(placed_key, _)| *placed_key == kids[0])
        .map(|(_, point)| point.x)
        .unwrap_or(start);
    let last = positions
        .iter()
        .find(|(placed_key, _)| *placed_key == kids[kids.len() - 1])
        .map(|(_, point)| point.x)
        .unwrap_or(*cursor_x - step_x);
    positions.push((key, Point::new((first + last) * 0.5, depth as f32 * step_y)));
    placed.insert(key);
}
