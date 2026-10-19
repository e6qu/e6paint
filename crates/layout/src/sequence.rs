//! Lane layout for sequence blocks: participants across the top, one row per message.

use e6_scene::Point;

/// Participants spread along x (`lane_step` apart, y = 0); message rows stack downward
/// (`row_step` apart, starting one step below the participant row). Pure, deterministic.
pub fn sequence_lanes(
    participants: &[String],
    message_count: usize,
    lane_step: f32,
    row_step: f32,
) -> (Vec<Point>, Vec<f32>) {
    let points = participants
        .iter()
        .enumerate()
        .map(|(index, _)| Point::new(index as f32 * lane_step, 0.0))
        .collect();
    let rows = (0..message_count)
        .map(|index| (index + 1) as f32 * row_step)
        .collect();
    (points, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_spread_participants_and_stack_rows() {
        let (points, rows) = sequence_lanes(&["a".into(), "b".into(), "c".into()], 3, 220.0, 90.0);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], Point::new(0.0, 0.0));
        assert_eq!(points[2], Point::new(440.0, 0.0));
        assert_eq!(rows, vec![90.0, 180.0, 270.0]);
    }

    #[test]
    fn empty_inputs_are_safe() {
        let (points, rows) = sequence_lanes(&[], 0, 100.0, 100.0);
        assert!(points.is_empty());
        assert!(rows.is_empty());
    }
}
