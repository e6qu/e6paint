//! Property tests for the layout engines (AGENTS testing pyramid: property
//! tests join with the layout milestone).
//!
//! Invariants every engine must hold for arbitrary inputs:
//! - layout succeeds for any well-formed graph,
//! - every node key appears exactly once in the output,
//! - all coordinates are finite,
//! - the layout is deterministic (same input, same output).

use e6_layout::{
    GridLayout, LayeredLayout, LayoutGraph, LayoutOutput, LayoutProvider, TreeLayout, charts,
};
use proptest::prelude::*;

fn graph_strategy() -> BoxedStrategy<LayoutGraph> {
    (1usize..12, 10.0_f64..200.0, 10.0_f64..200.0)
        .prop_flat_map(|(count, _w, _h)| {
            let keys: Vec<u64> = (0..count as u64).collect();
            let sizes = proptest::collection::vec((10.0_f64..200.0, 10.0_f64..200.0), count);
            let edge_pairs = proptest::collection::vec((0usize..count, 0usize..count), 0..10);
            (Just(keys), sizes, edge_pairs)
        })
        .prop_filter(
            "edges must connect distinct declared nodes",
            |(_, _, edge_pairs)| edge_pairs.iter().all(|(from, to)| from != to),
        )
        .prop_map(|(keys, sizes, edge_pairs)| {
            let mut graph = LayoutGraph::default();
            for (key, (w, h)) in keys.iter().zip(sizes.iter()) {
                graph = graph.node(*key, e6_scene::Size::new(*w as f32, *h as f32));
            }
            for (from, to) in edge_pairs {
                graph.edges.push(e6_layout::LayoutEdge {
                    from: keys[from],
                    to: keys[to],
                });
            }
            graph
        })
        .boxed()
}

fn check_output(graph: &LayoutGraph, out: &LayoutOutput) {
    // Every key placed exactly once.
    let mut keys: Vec<u64> = out.positions.iter().map(|(key, _)| *key).collect();
    let mut expected: Vec<u64> = graph.nodes.iter().map(|node| node.key).collect();
    keys.sort();
    expected.sort();
    assert_eq!(keys, expected, "layout must place every key exactly once");

    // All coordinates finite.
    for (_, point) in &out.positions {
        assert!(
            point.x.is_finite() && point.y.is_finite(),
            "non-finite layout coordinate"
        );
    }
}

/// A forest: keys are ordered so every edge points strictly forward, which makes
/// the graph acyclic by construction (parent always precedes child).
fn tree_graph_strategy() -> BoxedStrategy<LayoutGraph> {
    (2usize..14)
        .prop_flat_map(|count| {
            let sizes = proptest::collection::vec((10.0_f64..200.0, 10.0_f64..200.0), count);
            let edges = proptest::collection::vec(
                (0usize..count, 0usize..count),
                0..count.saturating_sub(1),
            );
            (Just(count), sizes, edges)
        })
        .prop_filter(
            "tree edges must point strictly forward (parent precedes child)",
            |(count, _, edges)| edges.iter().all(|(from, to)| from < to && *to < *count),
        )
        .prop_map(|(count, sizes, edges)| {
            let mut graph = LayoutGraph::default();
            for (key, (w, h)) in (0..count as u64).zip(sizes.iter()) {
                graph = graph.node(key, e6_scene::Size::new(*w as f32, *h as f32));
            }
            for (from, to) in edges {
                graph.edges.push(e6_layout::LayoutEdge {
                    from: from as u64,
                    to: to as u64,
                });
            }
            graph
        })
        .boxed()
}

proptest! {
    #[test]
    fn layered_layout_is_total_and_deterministic(graph in graph_strategy()) {
        let engine = LayeredLayout {
            node_size: e6_scene::Size::new(120.0, 60.0),
            gap: e6_scene::Size::new(40.0, 40.0),
            direction: e6_layout::Direction::LeftRight,
        };
        let first = engine.layout(&graph)?;
        check_output(&graph, &first);
        let second = engine.layout(&graph)?;
        prop_assert_eq!(first.positions, second.positions, "layout is not deterministic");
    }

    #[test]
    fn grid_layout_is_total_and_deterministic(graph in graph_strategy()) {
        let engine = GridLayout {
            columns: 3,
            cell: e6_scene::Size::new(100.0, 80.0),
        };
        let first = engine.layout(&graph)?;
        check_output(&graph, &first);
        let second = engine.layout(&graph)?;
        prop_assert_eq!(first.positions, second.positions);
    }

    #[test]
    fn tree_layout_is_total_and_deterministic(graph in tree_graph_strategy()) {
        let engine = TreeLayout {
            node_size: e6_scene::Size::new(140.0, 50.0),
            gap: e6_scene::Size::new(30.0, 30.0),
        };
        // Forests lay out totally and deterministically; multi-parent graphs are
        // rejected loudly instead of duplicating nodes.
        match engine.layout(&graph) {
            Ok(first) => {
                check_output(&graph, &first);
                let second = engine.layout(&graph)?;
                prop_assert_eq!(first.positions, second.positions, "tree layout is not deterministic");
            }
            Err(e6_layout::LayoutError::MultipleParents { .. }) => {}
            Err(err) => {
                prop_assert!(false, "unexpected layout error: {err:?}");
            }
        }
    }

    #[test]
    fn pie_sectors_tile_the_circle(
        values in proptest::collection::vec(1.0_f32..100.0, 1..9),
        radius in 40.0_f32..300.0,
    ) {
        let center = e6_scene::Point::new(0.0, 0.0);
        let total: f32 = values.iter().sum();
        let mut start = -std::f32::consts::FRAC_PI_2;
        let mut swept = 0.0_f32;
        for value in &values {
            let sweep = value / total * std::f32::consts::TAU;
            let points = charts::pie_sector(center, radius, start, sweep, 24);
            prop_assert!(points.len() >= 3, "sector polygon needs points");
            for point in &points {
                let distance = ((point.x - center.x).powi(2) + (point.y - center.y).powi(2)).sqrt();
                prop_assert!(
                    distance <= radius + 0.5,
                    "sector point {point:?} escapes radius {radius}"
                );
            }
            start += sweep;
            swept += sweep;
        }
        prop_assert!((swept - std::f32::consts::TAU).abs() < 1e-3, "sectors must tile the circle");
    }
}
