//! Expansion: a parsed mol document → placed, frame-ready content.
//!
//! One-way per the design: parse → layout → [`Expansion`] (nodes with positions, edges
//! with endpoint pairs). The caller stamps these into the board; re-apply is a later
//! milestone that replays provenance.

use std::collections::HashMap;

use e6_layout::{
    Direction as LayoutDirection, LayeredLayout, LayoutGraph, LayoutProvider, TreeLayout,
};
use e6_scene::{Point, Size};

use crate::ast::{BlockKind, Direction, MolDocument, MolLine};

/// Sizing for expansion.
#[derive(Debug, Clone)]
pub struct ExpandOptions {
    /// Flow/state node box.
    pub node: Size,
    /// Mind node box (smaller).
    pub mind_node: Size,
    /// Spacing between nodes.
    pub gap: Size,
}

impl Default for ExpandOptions {
    fn default() -> Self {
        Self {
            node: Size::new(160.0, 64.0),
            mind_node: Size::new(120.0, 48.0),
            gap: Size::new(60.0, 60.0),
        }
    }
}

/// One expanded block.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedBlock {
    /// Title as written, or the kind keyword when untitled.
    pub title: String,
    pub kind: BlockKind,
    pub direction: Direction,
    pub nodes: Vec<ExpandedNode>,
    pub edges: Vec<ExpandedEdge>,
}

/// A placed node: id, display label, top-left position, size, and optional polygon
/// payload (sector geometry for chart families).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedNode {
    pub id: String,
    pub label: String,
    pub position: Point,
    pub size: Size,
    pub points: Vec<Point>,
}

/// A placed edge: endpoint ids, optional label, and the two connection points.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub points: [Point; 2],
}

/// Everything an expansion produces, block by block in source order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Expansion {
    pub blocks: Vec<ExpandedBlock>,
}

/// Expansion failures (layout engines can reject graphs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpandError {
    Layout(String),
    /// A chart row was malformed (e.g. unparseable value or date).
    BadRow {
        row: usize,
        why: String,
    },
}

impl std::fmt::Display for ExpandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(why) => write!(f, "layout failed: {why}"),
            Self::BadRow { row, why } => write!(f, "bad chart row {row}: {why}"),
        }
    }
}

impl std::error::Error for ExpandError {}

/// Expand every block. An empty document expands to an empty expansion.
pub fn expand(document: &MolDocument, options: &ExpandOptions) -> Result<Expansion, ExpandError> {
    let mut blocks = Vec::with_capacity(document.blocks.len());
    for block in &document.blocks {
        blocks.push(expand_block(block, options)?);
    }
    Ok(Expansion { blocks })
}

/// Display label: explicit label wins, then member text, then the bare id.
fn node_label(
    labels: &HashMap<String, String>,
    member_text: &HashMap<String, String>,
    id: &str,
) -> String {
    if let Some(label) = labels.get(id).filter(|label| !label.is_empty()) {
        return label.clone();
    }
    if let Some(text) = member_text.get(id) {
        return format!("{id}: {text}");
    }
    id.to_owned()
}

fn expand_block(
    block: &crate::ast::MolBlock,
    options: &ExpandOptions,
) -> Result<ExpandedBlock, ExpandError> {
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut edges: Vec<(String, String, Option<String>)> = Vec::new();
    let mut members: Vec<(String, String)> = Vec::new();
    let mut seq_msgs: Vec<(String, String, bool, Option<String>)> = Vec::new();
    let mut git_ops: Vec<(crate::ast::GitOpKind, String)> = Vec::new();

    for line in &block.lines {
        match line {
            MolLine::GitOp { op, arg } => {
                git_ops.push((*op, arg.clone()));
            }
            MolLine::NodeDecl { id, label } => {
                if !labels.contains_key(id) {
                    order.push(id.clone());
                }
                if !label.is_empty() {
                    labels.insert(id.clone(), label.clone());
                }
            }
            MolLine::Members { id, text } => {
                if !labels.contains_key(id) {
                    labels.insert(id.clone(), String::new());
                    order.push(id.clone());
                }
                members.push((id.clone(), text.clone()));
            }
            MolLine::Edge { from, to, label } => {
                for node in [from, to] {
                    if !labels.contains_key(&node.id) {
                        order.push(node.id.clone());
                    }
                    if let Some(node_label) = &node.label {
                        labels.insert(node.id.clone(), node_label.clone());
                    }
                }
                edges.push((from.id.clone(), to.id.clone(), label.clone()));
            }
            MolLine::SeqMsg {
                from,
                to,
                dashed,
                label,
            } => {
                for id in [from, to] {
                    if !labels.contains_key(id) {
                        labels.insert(id.clone(), String::new());
                        order.push(id.clone());
                    }
                }
                seq_msgs.push((from.clone(), to.clone(), *dashed, label.clone()));
            }
            MolLine::MindLink { chain } => {
                for id in chain {
                    if !labels.contains_key(id) {
                        order.push(id.clone());
                    }
                }
                for pair in chain.windows(2) {
                    edges.push((pair[0].clone(), pair[1].clone(), None));
                }
            }
            MolLine::Direction { .. } => {}
        }
    }

    // Member lists fold into node labels as `Title: members` (rendered by the text
    // overlay; the layout box is shared with plain nodes).
    let mut member_text: HashMap<String, String> = HashMap::new();
    for (id, text) in &members {
        let entry = member_text.entry(id.clone()).or_default();
        if !entry.is_empty() {
            entry.push_str("; ");
        }
        entry.push_str(text);
    }

    let node_size = match block.kind {
        BlockKind::Mind => options.mind_node,
        _ => options.node,
    };
    // Layout keys are positions in the declaration order — exact, no hashing.
    let key_of: HashMap<String, u64> = order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), index as u64))
        .collect();

    // Sequence blocks take the dedicated lane layout: participants across the top,
    // messages stacked below, endpoints aligned to participant lane centers.
    if block.kind == BlockKind::Seq {
        let lane_step = node_size.width + options.gap.width;
        let row_step = node_size.height + options.gap.height;
        let (participant_points, row_ys) =
            e6_layout::sequence_lanes(&order, seq_msgs.len(), lane_step, row_step);
        let x_of: HashMap<String, f32> = order
            .iter()
            .cloned()
            .zip(participant_points.iter().map(|p| p.x))
            .collect();
        let nodes: Vec<ExpandedNode> = order
            .iter()
            .zip(participant_points.iter())
            .map(|(id, position)| ExpandedNode {
                id: id.clone(),
                label: node_label(&labels, &member_text, id),
                position: *position,
                size: node_size,
                points: Vec::new(),
            })
            .collect();
        let edges = seq_msgs
            .iter()
            .enumerate()
            .filter_map(|(index, (from, to, _dashed, label))| {
                let x_from = *x_of.get(from)?;
                let x_to = *x_of.get(to)?;
                let y = *row_ys.get(index)?;
                Some(ExpandedEdge {
                    from: from.clone(),
                    to: to.clone(),
                    label: label.clone(),
                    points: [
                        Point::new(x_from + node_size.width * 0.5, y),
                        Point::new(x_to + node_size.width * 0.5, y),
                    ],
                })
            })
            .collect();
        return Ok(ExpandedBlock {
            title: block
                .title
                .clone()
                .unwrap_or_else(|| block.kind.keyword().to_owned()),
            kind: block.kind,
            direction: block.direction,
            nodes,
            edges,
        });
    }

    // Chart families: timeline / pie / gantt / git rows drive deterministic geometry.
    if matches!(
        block.kind,
        BlockKind::Timeline | BlockKind::Pie | BlockKind::Gantt | BlockKind::Git
    ) {
        return expand_chart_block(block, options, &members, &git_ops);
    }

    let mut graph = LayoutGraph::default();
    for id in &order {
        graph = graph.node(key_of[id], node_size);
    }
    for (from, to, _) in &edges {
        graph = graph.edge(key_of[from], key_of[to]);
    }

    let output = match block.kind {
        BlockKind::Mind => TreeLayout {
            node_size,
            gap: options.gap,
        }
        .layout(&graph)
        .map_err(|err| ExpandError::Layout(err.to_string()))?,
        _ => {
            let direction = match block.direction {
                Direction::LeftRight => LayoutDirection::LeftRight,
                Direction::TopDown => LayoutDirection::TopDown,
            };
            LayeredLayout {
                node_size,
                gap: options.gap,
                direction,
            }
            .layout(&graph)
            .map_err(|err| ExpandError::Layout(err.to_string()))?
        }
    };

    let lookup: HashMap<u64, Point> = output.positions.iter().copied().collect();
    let key_by_index = |id: &str| key_of.get(id).copied();
    let nodes: Vec<ExpandedNode> = order
        .iter()
        .filter_map(|id| {
            key_by_index(id)
                .and_then(|key| lookup.get(&key))
                .map(|position| ExpandedNode {
                    id: id.clone(),
                    label: node_label(&labels, &member_text, id),
                    position: *position,
                    size: node_size,
                    points: Vec::new(),
                })
        })
        .collect();
    let by_id: HashMap<&str, &ExpandedNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut placed_edges = Vec::with_capacity(edges.len());
    for (from, to, label) in edges {
        let (Some(a), Some(b)) = (by_id.get(from.as_str()), by_id.get(to.as_str())) else {
            continue;
        };
        placed_edges.push(ExpandedEdge {
            from,
            to,
            label,
            points: connection_points(a, b, block.direction),
        });
    }

    Ok(ExpandedBlock {
        title: block
            .title
            .clone()
            .unwrap_or_else(|| block.kind.keyword().to_owned()),
        kind: block.kind,
        direction: block.direction,
        nodes,
        edges: placed_edges,
    })
}

fn connection_points(a: &ExpandedNode, b: &ExpandedNode, direction: Direction) -> [Point; 2] {
    let center = |node: &ExpandedNode| {
        Point::new(
            node.position.x + node.size.width * 0.5,
            node.position.y + node.size.height * 0.5,
        )
    };
    let a_center = center(a);
    let b_center = center(b);
    match direction {
        Direction::LeftRight => [
            Point::new(a.position.x + a.size.width, a_center.y),
            Point::new(b.position.x, b_center.y),
        ],
        Direction::TopDown => [
            Point::new(a_center.x, a.position.y + a.size.height),
            Point::new(b_center.x, b.position.y),
        ],
    }
}

// Edge labels are carried in the AST; wave 1 places them as small text elements next to
// the connector midpoint in the app layer.

/// Parse `YYYY-MM-DD` into a day number (days from civil algorithm, proleptic
/// Gregorian). Pure and deterministic.
pub fn parse_date(word: &str) -> Option<i64> {
    let mut parts = word.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Howard Hinnant's days-from-civil (proleptic Gregorian, day 0 = 1970-01-01).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse a duration suffix: `Nd` days, `Nw` weeks (N days * 7).
fn parse_duration(word: &str) -> Option<f32> {
    let (number, unit) = word.split_at(word.len().saturating_sub(1));
    let value: f32 = number.parse().ok()?;
    match unit {
        "d" => Some(value),
        "w" => Some(value * 7.0),
        _ => None,
    }
}

/// Chart-family expansion: timeline rows stack, pie slices polygonize, gantt bars lay
/// out on a day axis, git ops walk branch lanes. All deterministic; no graph engine.
fn expand_chart_block(
    block: &crate::ast::MolBlock,
    options: &ExpandOptions,
    members: &[(String, String)],
    git_ops: &[(crate::ast::GitOpKind, String)],
) -> Result<ExpandedBlock, ExpandError> {
    use crate::ast::BlockKind;
    use e6_layout as layout_crate;

    let title = block
        .title
        .clone()
        .unwrap_or_else(|| block.kind.keyword().to_owned());
    let mut nodes: Vec<ExpandedNode> = Vec::new();
    let mut edges: Vec<ExpandedEdge> = Vec::new();
    let row_step = options.gap.height.max(40.0);

    match block.kind {
        BlockKind::Timeline => {
            for (index, (period, event)) in members.iter().enumerate() {
                let y = index as f32 * row_step;
                nodes.push(ExpandedNode {
                    id: format!("{index}:period"),
                    label: period.clone(),
                    position: Point::new(0.0, y),
                    size: Size::new(140.0, 40.0),
                    points: Vec::new(),
                });
                nodes.push(ExpandedNode {
                    id: format!("{index}:event"),
                    label: event.clone(),
                    position: Point::new(260.0, y),
                    size: Size::new(320.0, 40.0),
                    points: Vec::new(),
                });
                if index > 0 {
                    edges.push(ExpandedEdge {
                        from: format!("{}:period", index - 1),
                        to: format!("{index}:period"),
                        label: None,
                        points: [Point::new(70.0, y - row_step + 40.0), Point::new(70.0, y)],
                    });
                }
            }
        }
        BlockKind::Pie => {
            let mut values: Vec<f32> = Vec::with_capacity(members.len());
            for (row_index, (_, text)) in members.iter().enumerate() {
                let value = text
                    .trim()
                    .parse::<f32>()
                    .map_err(|err| ExpandError::BadRow {
                        row: row_index,
                        why: format!("value `{text}` is not a number: {err}"),
                    })?;
                values.push(value);
            }
            let total: f32 = values.iter().sum();
            let radius = 140.0_f32;
            let center = Point::new(radius + 20.0, radius + 20.0);
            let mut start = -std::f32::consts::FRAC_PI_2;
            for ((label, _), value) in members.iter().zip(&values) {
                let sweep = if total > 0.0 {
                    value / total * std::f32::consts::TAU
                } else {
                    0.0
                };
                let polygon = layout_crate::pie_sector(center, radius, start, sweep, 24);
                let anchor = layout_crate::pie_label_anchor(center, radius * 0.66, start, sweep);
                nodes.push(ExpandedNode {
                    id: format!("slice:{label}"),
                    label: label.clone(),
                    position: Point::new(center.x - radius, center.y - radius),
                    size: Size::new(radius * 2.0, radius * 2.0),
                    points: polygon,
                });
                nodes.push(ExpandedNode {
                    id: format!("slice-label:{label}"),
                    label: format!("{label} {:.0}", value.max(0.0)),
                    position: Point::new(anchor.x - 40.0, anchor.y - 12.0),
                    size: Size::new(80.0, 24.0),
                    points: Vec::new(),
                });
                start += sweep;
            }
        }
        BlockKind::Gantt => {
            let mut starts: Vec<f32> = Vec::new();
            let mut durations: Vec<f32> = Vec::new();
            for (row_index, (_, text)) in members.iter().enumerate() {
                let bad = |why: String| ExpandError::BadRow {
                    row: row_index,
                    why,
                };
                let mut parts = text.split(',');
                let date_word = parts.next().unwrap_or_default().trim().to_owned();
                let start = match parse_date(&date_word) {
                    Some(days) => days as f32,
                    None => {
                        return Err(bad(format!("start date `{date_word}` is invalid")));
                    }
                };
                let duration_word = parts.next().unwrap_or_default().trim().to_owned();
                let duration = match parse_duration(&duration_word) {
                    Some(days) => days,
                    None => {
                        return Err(bad(format!(
                            "duration `{duration_word}` is invalid (use Nd or Nw)"
                        )));
                    }
                };
                starts.push(start);
                durations.push(duration);
            }
            let origin = starts.iter().cloned().reduce(f32::min).unwrap_or(0.0);
            let day_width = 6.0;
            for (index, ((task, _), start)) in members.iter().zip(&starts).enumerate() {
                let bars = layout_crate::gantt_bars(
                    &[start - origin],
                    &[durations[index]],
                    40.0,
                    day_width,
                    &[index as f32 * row_step],
                    row_step * 0.6,
                );
                let (position, size) = bars[0];
                nodes.push(ExpandedNode {
                    id: format!("{index}:task"),
                    label: task.clone(),
                    position,
                    size,
                    points: Vec::new(),
                });
            }
        }
        BlockKind::Git => {
            let mut branches: Vec<String> = vec!["main".into()];
            let mut current = "main".to_owned();
            let mut commit_index = 0usize;
            let lane_step = row_step;
            let commit_step = 70.0;
            let mut lane_tips: HashMap<String, Point> = HashMap::new();
            for (op, arg) in git_ops {
                let lane = branch_lane(&branches, &current);
                let y = lane as f32 * lane_step;
                match op {
                    crate::ast::GitOpKind::Commit => {
                        let id = format!("c{commit_index}");
                        commit_index += 1;
                        let x = commit_index as f32 * commit_step;
                        nodes.push(ExpandedNode {
                            id: id.clone(),
                            label: arg.clone(),
                            position: Point::new(x - 12.0, y - 12.0),
                            size: Size::new(24.0, 24.0),
                            points: Vec::new(),
                        });
                        lane_tips.insert(current.clone(), Point::new(x, y));
                    }
                    crate::ast::GitOpKind::Branch => {
                        if !branches.contains(arg) {
                            branches.push(arg.clone());
                        }
                    }
                    crate::ast::GitOpKind::Checkout => {
                        current = arg.clone();
                    }
                    crate::ast::GitOpKind::Merge => {
                        let from: Option<Point> = lane_tips.get(arg).copied();
                        let lane = branch_lane(&branches, &current);
                        let x = commit_index as f32 * commit_step + commit_step;
                        commit_index += 1;
                        if let Some(tip) = from {
                            edges.push(ExpandedEdge {
                                from: arg.clone(),
                                to: current.clone(),
                                label: Some("merge".into()),
                                points: [tip, Point::new(x, lane as f32 * lane_step)],
                            });
                        }
                        let merge_id = format!("m{commit_index}");
                        nodes.push(ExpandedNode {
                            id: merge_id,
                            label: format!("merge {arg}"),
                            position: Point::new(x - 12.0, y - 12.0),
                            size: Size::new(24.0, 24.0),
                            points: Vec::new(),
                        });
                        lane_tips.insert(current.clone(), Point::new(x, lane as f32 * lane_step));
                    }
                }
            }
        }
        _ => {}
    }

    Ok(ExpandedBlock {
        title,
        kind: block.kind,
        direction: block.direction,
        nodes,
        edges,
    })
}

fn branch_lane(branches: &[String], name: &str) -> usize {
    branches.iter().position(|b| b == name).unwrap_or(0)
}
