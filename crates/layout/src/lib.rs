//! Pure layout engines: layered, tree, lane/slab, grid, chart, and the shared
//! orthogonal router. Every engine is deterministic — identical inputs and seed produce
//! identical output (property-tested).
//!
//! Engines are reachable only through [`LayoutProvider`], which keeps the door open for a
//! JS-backed provider without any call-site changes.

pub mod charts;
pub mod layered;
pub mod sequence;
pub mod tree;

pub use charts::{gantt_bars, pie_label_anchor, pie_sector, timeline_rows};
pub use layered::{Direction, LayeredLayout};
pub use sequence::sequence_lanes;
pub use tree::TreeLayout;

use std::collections::HashSet;

use e6_scene::{Point, Size};

pub(crate) fn validate(graph: &LayoutGraph) -> Result<(), LayoutError> {
    if graph.nodes.is_empty() {
        return Err(LayoutError::EmptyGraph);
    }
    let mut seen = HashSet::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        if !seen.insert(node.key) {
            return Err(LayoutError::DuplicateNode { key: node.key });
        }
    }
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        for key in [edge.from, edge.to] {
            if !seen.contains(&key) {
                return Err(LayoutError::UnknownEdgeEndpoint {
                    edge: edge_index,
                    key,
                });
            }
        }
    }
    Ok(())
}

/// A layout input graph. Node keys are caller-assigned and stable within one request.
#[derive(Debug, Clone, Default)]
pub struct LayoutGraph {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
}

impl LayoutGraph {
    pub fn node(mut self, key: u64, size: Size) -> Self {
        self.nodes.push(LayoutNode { key, size });
        self
    }

    pub fn edge(mut self, from: u64, to: u64) -> Self {
        self.edges.push(LayoutEdge { from, to });
        self
    }
}

/// A node to place, with its intrinsic size.
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub key: u64,
    pub size: Size,
}

/// A directed edge between node keys.
#[derive(Debug, Clone)]
pub struct LayoutEdge {
    pub from: u64,
    pub to: u64,
}

/// Placed node top-left positions, keyed by node key.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutOutput {
    pub positions: Vec<(u64, Point)>,
}

/// Reasons a layout request cannot be satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// The graph declares no nodes.
    EmptyGraph,
    /// Two nodes share a key.
    DuplicateNode { key: u64 },
    /// An edge references an undeclared node key.
    UnknownEdgeEndpoint { edge: usize, key: u64 },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyGraph => write!(f, "layout graph declares no nodes"),
            Self::DuplicateNode { key } => write!(f, "duplicate node key {key}"),
            Self::UnknownEdgeEndpoint { edge, key } => {
                write!(f, "edge {edge} references undeclared node key {key}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// The layout seam. Implementations are the pure-Rust engines; a JS-backed provider may
/// slot in behind this trait without call-site changes.
pub trait LayoutProvider {
    fn layout(&self, graph: &LayoutGraph) -> Result<LayoutOutput, LayoutError>;
}

/// Row-major grid placement. Proves the seam and serves as the trivial test provider.
#[derive(Debug, Clone)]
pub struct GridLayout {
    pub columns: u32,
    pub cell: Size,
}

impl LayoutProvider for GridLayout {
    fn layout(&self, graph: &LayoutGraph) -> Result<LayoutOutput, LayoutError> {
        validate(graph)?;
        let columns = self.columns.max(1);
        let mut positions = Vec::with_capacity(graph.nodes.len());
        for (index, node) in graph.nodes.iter().enumerate() {
            let index = index as u32;
            let column = index % columns;
            let row = index / columns;
            positions.push((
                node.key,
                Point::new(
                    column as f32 * self.cell.width,
                    row as f32 * self.cell.height,
                ),
            ));
        }
        Ok(LayoutOutput { positions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> LayoutGraph {
        LayoutGraph::default()
            .node(1, Size::new(10.0, 10.0))
            .node(2, Size::new(10.0, 10.0))
            .node(3, Size::new(10.0, 10.0))
            .edge(1, 2)
            .edge(2, 3)
    }

    #[test]
    fn grid_places_row_major() -> Result<(), Box<dyn std::error::Error>> {
        let out = GridLayout {
            columns: 2,
            cell: Size::new(100.0, 50.0),
        }
        .layout(&graph())?;
        let at = |key: u64| {
            out.positions
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, p)| *p)
        };
        assert_eq!(at(1), Some(Point::new(0.0, 0.0)));
        assert_eq!(at(2), Some(Point::new(100.0, 0.0)));
        assert_eq!(at(3), Some(Point::new(0.0, 50.0)));
        Ok(())
    }

    #[test]
    fn validation_errors() -> Result<(), Box<dyn std::error::Error>> {
        let layout = GridLayout {
            columns: 1,
            cell: Size::new(1.0, 1.0),
        };
        assert_eq!(
            layout.layout(&LayoutGraph::default()),
            Err(LayoutError::EmptyGraph)
        );

        let dup = LayoutGraph::default()
            .node(1, Size::new(1.0, 1.0))
            .node(1, Size::new(1.0, 1.0));
        assert_eq!(
            layout.layout(&dup),
            Err(LayoutError::DuplicateNode { key: 1 })
        );

        let dangling = LayoutGraph::default()
            .node(1, Size::new(1.0, 1.0))
            .edge(1, 9);
        assert_eq!(
            layout.layout(&dangling),
            Err(LayoutError::UnknownEdgeEndpoint { edge: 0, key: 9 })
        );
        Ok(())
    }
}
