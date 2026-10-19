//! Deterministic geometry helpers for chart families (timeline/pie/gantt/git).

use e6_scene::{Point, Size};

/// Timeline rows: each entry stacks one `row_step` below the previous.
pub fn timeline_rows(count: usize, row_step: f32) -> Vec<f32> {
    (0..count).map(|index| index as f32 * row_step).collect()
}

/// Sector polygon for a pie slice: `segments`-point arc approximation from
/// `start_angle` sweeping `sweep_angle` (radians), closed back to the center.
pub fn pie_sector(
    center: Point,
    radius: f32,
    start_angle: f32,
    sweep_angle: f32,
    segments: usize,
) -> Vec<Point> {
    let segments = segments.max(2);
    let mut out = Vec::with_capacity(segments + 2);
    out.push(center);
    for step in 0..=segments {
        let angle = start_angle + sweep_angle * (step as f32 / segments as f32);
        out.push(Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    out
}

/// Label anchor for a pie slice: along the mid-angle at `label_radius`.
pub fn pie_label_anchor(center: Point, radius: f32, start_angle: f32, sweep_angle: f32) -> Point {
    let mid = start_angle + sweep_angle * 0.5;
    Point::new(center.x + radius * mid.cos(), center.y + radius * mid.sin())
}

/// Gantt bars: rows stack by index; x/width come from pre-computed day offsets.
pub fn gantt_bars(
    start_days: &[f32],
    duration_days: &[f32],
    origin_x: f32,
    day_width: f32,
    row_y: &[f32],
    bar_height: f32,
) -> Vec<(Point, Size)> {
    use e6_scene::Size;
    start_days
        .iter()
        .zip(duration_days)
        .zip(row_y)
        .map(|((start, duration), y)| {
            (
                Point::new(origin_x + start * day_width, *y),
                Size::new(duration.max(1.0) * day_width, bar_height),
            )
        })
        .collect()
}

/// Git lane y positions, one row per branch (main first).
pub fn git_lanes(lane_count: usize, lane_step: f32) -> Vec<f32> {
    (0..lane_count)
        .map(|index| index as f32 * lane_step)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_rows_stack() {
        assert_eq!(timeline_rows(3, 90.0), vec![0.0, 90.0, 180.0]);
        assert!(timeline_rows(0, 90.0).is_empty());
    }

    #[test]
    fn pie_sector_closes_at_start_angle() -> Result<(), Box<dyn std::error::Error>> {
        let polygon = pie_sector(Point::new(0.0, 0.0), 50.0, 0.0, std::f32::consts::TAU, 8);
        assert_eq!(polygon.len(), 10, "center + 9 boundary points");
        let first = polygon[1];
        let last = *polygon.last().ok_or("polygon non-empty")?;
        assert!(
            (first.x - last.x).abs() < 0.01 && (first.y - last.y).abs() < 0.01,
            "full sweep closes the arc"
        );
        Ok(())
    }

    #[test]
    fn gantt_bars_scale_by_day_width() {
        let bars = gantt_bars(&[0.0, 5.0], &[2.0, 3.0], 100.0, 10.0, &[0.0, 40.0], 20.0);
        assert_eq!(bars[0].0, Point::new(100.0, 0.0));
        assert_eq!(bars[0].1, Size::new(20.0, 20.0));
        assert_eq!(bars[1].0, Point::new(150.0, 40.0));
        assert_eq!(bars[1].1, Size::new(30.0, 20.0));
    }
}
