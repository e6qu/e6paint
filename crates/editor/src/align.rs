//! Alignment and distribution math over a multi-selection.
//!
//! Pure: callers hand in `(id, bounds)` pairs and get back per-id translation deltas to
//! apply (the document's `translate` moves point payloads together with bounds).

use e6_scene::{Aabb, ElementId, Point};
/// How to arrange the selection inside its union bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    CenterH,
    Right,
    Top,
    CenterV,
    Bottom,
    DistributeH,
    DistributeV,
}

/// Minimum selection size for a mode to be meaningful.
pub fn min_selection(mode: AlignMode) -> usize {
    match mode {
        AlignMode::DistributeH | AlignMode::DistributeV => 3,
        _ => 2,
    }
}

/// Translation deltas realizing `mode` over `items`.
///
/// Aligns reference the union bounds of the selection; distributes spread element
/// centers evenly between the outermost centers along the axis (outermost stay put).
/// Fewer items than [`min_selection`] yields no deltas; overlapping positions may
/// yield zero deltas, which is a valid outcome, not an error.
pub fn compute_deltas(items: &[(ElementId, Aabb)], mode: AlignMode) -> Vec<(ElementId, Point)> {
    if items.len() < min_selection(mode) {
        return Vec::new();
    }
    let mut bounds_iter = items.iter().map(|(_, bounds)| *bounds);
    let Some(first) = bounds_iter.next() else {
        return Vec::new();
    };
    let union = bounds_iter.fold(first, Aabb::union);
    match mode {
        AlignMode::Left => shift_each(items, |b| Point::new(union.min.x - b.min.x, 0.0)),
        AlignMode::Right => shift_each(items, |b| Point::new(union.max.x - b.max.x, 0.0)),
        AlignMode::CenterH => shift_each(items, |b| {
            Point::new(
                (union.min.x + union.max.x) * 0.5 - (b.min.x + b.max.x) * 0.5,
                0.0,
            )
        }),
        AlignMode::Top => shift_each(items, |b| Point::new(0.0, union.min.y - b.min.y)),
        AlignMode::Bottom => shift_each(items, |b| Point::new(0.0, union.max.y - b.max.y)),
        AlignMode::CenterV => shift_each(items, |b| {
            Point::new(
                0.0,
                (union.min.y + union.max.y) * 0.5 - (b.min.y + b.max.y) * 0.5,
            )
        }),
        AlignMode::DistributeH => distribute(items, union, true),
        AlignMode::DistributeV => distribute(items, union, false),
    }
}

fn shift_each(
    items: &[(ElementId, Aabb)],
    delta: impl Fn(&Aabb) -> Point,
) -> Vec<(ElementId, Point)> {
    items
        .iter()
        .map(|(id, bounds)| (*id, delta(bounds)))
        .collect()
}

fn distribute(
    items: &[(ElementId, Aabb)],
    union: Aabb,
    horizontal: bool,
) -> Vec<(ElementId, Point)> {
    let mut sorted: Vec<(ElementId, Aabb)> = items.to_vec();
    sorted.sort_by(|a, b| center(a.1, horizontal).total_cmp(&center(b.1, horizontal)));
    let count = sorted.len();
    let (lo, hi) = if horizontal {
        (union.min.x, union.max.x)
    } else {
        (union.min.y, union.max.y)
    };
    let mut deltas = Vec::with_capacity(count);
    for (index, (id, bounds)) in sorted.iter().enumerate() {
        if index == 0 || index + 1 == count {
            deltas.push((*id, Point::ZERO));
            continue;
        }
        let span = hi - lo;
        let target_center = lo + span * (index as f32) / ((count - 1) as f32);
        let current_center = center(*bounds, horizontal);
        let delta = target_center - current_center;
        deltas.push((
            *id,
            if horizontal {
                Point::new(delta, 0.0)
            } else {
                Point::new(0.0, delta)
            },
        ));
    }
    deltas
}

fn center(bounds: Aabb, horizontal: bool) -> f32 {
    if horizontal {
        (bounds.min.x + bounds.max.x) * 0.5
    } else {
        (bounds.min.y + bounds.max.y) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> ElementId {
        ElementId::from_raw(raw)
    }

    fn box_at(x: f32, y: f32) -> Aabb {
        use e6_scene::Size;
        Aabb::from_min_size(Point::new(x, y), Size::new(10.0, 10.0))
    }

    fn items() -> Vec<(ElementId, Aabb)> {
        vec![
            (id(1), box_at(0.0, 0.0)),
            (id(2), box_at(50.0, 20.0)),
            (id(3), box_at(100.0, 5.0)),
        ]
    }

    fn delta_for(
        deltas: &[(ElementId, Point)],
        raw: u64,
    ) -> Result<Point, Box<dyn std::error::Error>> {
        Ok(deltas
            .iter()
            .find(|(candidate, _)| candidate.raw() == raw)
            .map(|(_, delta)| *delta)
            .ok_or("id present in deltas")?)
    }

    #[test]
    fn too_few_items_yield_no_deltas() {
        let two = &items()[..2];
        assert!(compute_deltas(two, AlignMode::DistributeH).is_empty());
        assert!(compute_deltas(&[], AlignMode::Left).is_empty());
    }

    #[test]
    fn align_left_moves_every_min_to_the_union_left() -> Result<(), Box<dyn std::error::Error>> {
        let deltas = compute_deltas(&items(), AlignMode::Left);
        assert_eq!(delta_for(&deltas, 1)?, Point::ZERO);
        assert_eq!(delta_for(&deltas, 2)?, Point::new(-50.0, 0.0));
        assert_eq!(delta_for(&deltas, 3)?, Point::new(-100.0, 0.0));
        Ok(())
    }

    #[test]
    fn align_top_moves_every_min_to_the_union_top() -> Result<(), Box<dyn std::error::Error>> {
        let deltas = compute_deltas(&items(), AlignMode::Top);
        assert_eq!(delta_for(&deltas, 2)?, Point::new(0.0, -20.0));
        assert_eq!(delta_for(&deltas, 3)?, Point::new(0.0, -5.0));
        Ok(())
    }

    #[test]
    fn distribute_h_spreads_centers_evenly() -> Result<(), Box<dyn std::error::Error>> {
        let deltas = compute_deltas(&items(), AlignMode::DistributeH);
        // Sorted by center: id1 @5, id2 @55, id3 @105 — already even, so only zero moves.
        assert_eq!(delta_for(&deltas, 1)?, Point::ZERO);
        assert_eq!(delta_for(&deltas, 2)?, Point::ZERO);
        assert_eq!(delta_for(&deltas, 3)?, Point::ZERO);

        // Bunched middle: centers 5, 25, 105 → middle target 55 → +30.
        let bunched = vec![
            (id(1), box_at(0.0, 0.0)),
            (id(2), box_at(20.0, 0.0)),
            (id(3), box_at(100.0, 0.0)),
        ];
        let deltas = compute_deltas(&bunched, AlignMode::DistributeH);
        assert_eq!(delta_for(&deltas, 2)?, Point::new(30.0, 0.0));
        Ok(())
    }

    #[test]
    fn distribute_v_orders_by_center() -> Result<(), Box<dyn std::error::Error>> {
        // Vertical centers: id1 @5, id3 @10, id2 @25 — id3 is the middle, moved to 15.
        let deltas = compute_deltas(&items(), AlignMode::DistributeV);
        assert_eq!(delta_for(&deltas, 1)?, Point::ZERO);
        assert_eq!(delta_for(&deltas, 3)?, Point::new(0.0, 5.0));
        assert_eq!(delta_for(&deltas, 2)?, Point::ZERO);
        Ok(())
    }
}

/// The outcome of one alignment-snap pass: adjusted drag deltas plus the world
/// coordinates of the guide lines to draw (vertical guides are x values).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapResult {
    pub dx: f32,
    pub dy: f32,
    pub guides_x: Vec<f32>,
    pub guides_y: Vec<f32>,
}

/// Miro-style snapping: while dragging `moving` by `(dx, dy)`, align its center
/// or edges to any other bounds' center/edges within `threshold` (screen px
/// converted to world units by the caller). One guide per axis — the nearest.
pub fn snap_delta(moving: Aabb, others: &[Aabb], dx: f32, dy: f32, threshold: f32) -> SnapResult {
    let mut result = SnapResult {
        dx,
        dy,
        ..Default::default()
    };

    let moving_center_x = moving.center().x + dx;
    let moving_center_y = moving.center().y + dy;
    let moving_left = moving.min.x + dx;
    let moving_right = moving.max.x + dx;
    let moving_top = moving.min.y + dy;
    let moving_bottom = moving.max.y + dy;

    let mut best_x: Option<(f32, f32, f32)> = None; // (|delta|, snap_dx, guide_x)
    let mut best_y: Option<(f32, f32, f32)> = None;

    for other in others {
        let center_x = other.center().x;
        for (moving_edge, other_edge) in [
            (moving_center_x, center_x),
            (moving_left, other.min.x),
            (moving_right, other.max.x),
            (moving_center_x, other.min.x),
            (moving_center_x, other.max.x),
            // Adjacent edges: the classic "snap flush against" case.
            (moving_right, other.min.x),
            (moving_left, other.max.x),
        ] {
            let delta = other_edge - moving_edge;
            if delta.abs() <= threshold && best_x.is_none_or(|(best, _, _)| delta.abs() < best) {
                best_x = Some((delta.abs(), delta, other_edge));
            }
        }

        let center_y = other.center().y;
        for (moving_edge, other_edge) in [
            (moving_center_y, center_y),
            (moving_top, other.min.y),
            (moving_bottom, other.max.y),
            (moving_center_y, other.min.y),
            (moving_center_y, other.max.y),
            (moving_bottom, other.min.y),
            (moving_top, other.max.y),
        ] {
            let delta = other_edge - moving_edge;
            if delta.abs() <= threshold && best_y.is_none_or(|(best, _, _)| delta.abs() < best) {
                best_y = Some((delta.abs(), delta, other_edge));
            }
        }
    }

    if let Some((_, snap, guide)) = best_x {
        result.dx += snap;
        result.guides_x.push(guide);
    }
    if let Some((_, snap, guide)) = best_y {
        result.dy += snap;
        result.guides_y.push(guide);
    }
    result
}

#[cfg(test)]
mod snap_tests {
    use super::*;

    fn rect(x: f32, y: f32) -> Aabb {
        Aabb::from_min_size(Point::new(x, y), e6_scene::Size::new(100.0, 80.0))
    }

    #[test]
    fn centers_snap_within_threshold() {
        // dx=4 lands the moving center at 54; the other's center is at 54.
        let moving = rect(0.0, 0.0);
        let others = [rect(4.0, 30.0)];
        let result = snap_delta(moving, &others, 4.0, 0.0, 6.0);
        assert_eq!(result.dx, 4.0, "already aligned: no extra snap");
        assert_eq!(result.guides_x, vec![54.0]);
        assert!(result.guides_y.is_empty());
    }

    #[test]
    fn beyond_threshold_no_snap() {
        let moving = rect(0.0, 0.0);
        let others = [rect(120.0, 0.0)];
        let result = snap_delta(moving, &others, 4.0, 0.0, 6.0);
        assert_eq!(result.dx, 4.0);
        assert!(result.guides_x.is_empty());
    }

    #[test]
    fn edges_snap_and_both_axes_independent() {
        // dx=0 puts the moving right edge at 100; the other's left edge is at 96
        // (4 px away): snaps so the edges touch.
        let moving = rect(0.0, 0.0);
        let others = [rect(96.0, 500.0)];
        let result = snap_delta(moving, &others, 0.0, 0.0, 6.0);
        assert_eq!(result.dx, -4.0);
        assert_eq!(result.guides_x, vec![96.0]);
    }

    #[test]
    fn nearest_candidate_wins() {
        // dx=3 lands the moving center at 53; candidates at 54 and 56: 54 wins.
        let moving = rect(0.0, 0.0);
        let others = [rect(4.0, 0.0), rect(6.0, 0.0)];
        let result = snap_delta(moving, &others, 3.0, 0.0, 6.0);
        assert_eq!(result.dx, 4.0);
        assert_eq!(result.guides_x, vec![54.0]);
    }
}
