//! Hand-drawn vector tool icons. No bundled icon fonts: every glyph is a few
//! painter primitives, so the binary stays free of third-party assets (AGENTS:
//! icon sets are user-loaded, never redistributed).

use e6_editor::Tool;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

/// Draw `tool`'s icon centered in `rect`, stroked/filled in `color`.
pub fn tool_icon(painter: &egui::Painter, rect: Rect, tool: Tool, color: Color32) {
    let c = rect.center();
    let s = rect.size().min_elem() * 0.55;
    let stroke = Stroke::new(1.6, color);
    match tool {
        Tool::Select => {
            // Cursor arrow, tilted like a pointer.
            let tip = Pos2::new(c.x - s * 0.35, c.y - s * 0.45);
            let pts = [
                tip,
                Pos2::new(tip.x + s * 0.18, tip.y + s * 0.78),
                Pos2::new(tip.x + s * 0.38, tip.y + s * 0.44),
                Pos2::new(tip.x + s * 0.74, tip.y + s * 0.52),
            ];
            painter.add(egui::Shape::convex_polygon(
                pts.to_vec(),
                color,
                Stroke::NONE,
            ));
        }
        Tool::Pan => {
            // Four-way move arrows.
            let arm = s * 0.42;
            let head = s * 0.16;
            for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                let tip = Pos2::new(c.x + dx * arm, c.y + dy * arm);
                let left = Pos2::new(
                    c.x + dx * (arm - head) - dy * head,
                    c.y + dy * (arm - head) + dx * head,
                );
                let right = Pos2::new(
                    c.x + dx * (arm - head) + dy * head,
                    c.y + dy * (arm - head) - dx * head,
                );
                painter.line_segment([c, tip], stroke);
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, left, right],
                    color,
                    Stroke::NONE,
                ));
            }
            painter.circle_filled(c, s * 0.08, color);
        }
        Tool::Pen => {
            // Diagonal pencil: body + tip.
            let dir = Vec2::new(0.7, 0.7).normalized();
            let side = Vec2::new(-dir.y, dir.x) * s * 0.14;
            let start = Pos2::new(c.x - dir.x * s * 0.45, c.y - dir.y * s * 0.45);
            let body_end = Pos2::new(c.x + dir.x * s * 0.22, c.y + dir.y * s * 0.22);
            let tip = Pos2::new(c.x + dir.x * s * 0.48, c.y + dir.y * s * 0.48);
            let body = vec![
                start + side,
                body_end + side,
                tip,
                body_end - side,
                start - side,
            ];
            painter.add(egui::Shape::convex_polygon(body, color, Stroke::NONE));
            painter.line_segment([start - side, start + side], Stroke::new(1.0, color));
        }
        Tool::Rect => {
            let half = s * 0.38;
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(half * 2.0)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Tool::Ellipse => {
            painter.circle_stroke(c, s * 0.4, stroke);
        }
        Tool::Connector => {
            let from = Pos2::new(c.x - s * 0.42, c.y + s * 0.3);
            let to = Pos2::new(c.x + s * 0.42, c.y - s * 0.3);
            painter.line_segment([from, to], stroke);
            let head = s * 0.18;
            let dir = Vec2::new(to.x - from.x, to.y - from.y).normalized();
            let left = Pos2::new(
                to.x - dir.x * head - dir.y * head,
                to.y - dir.y * head + dir.x * head,
            );
            let right = Pos2::new(
                to.x - dir.x * head + dir.y * head,
                to.y - dir.y * head - dir.x * head,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![to, left, right],
                color,
                Stroke::NONE,
            ));
        }
        Tool::Text => {
            // A bold T: top bar + stem.
            let w = s * 0.7;
            let bar_h = s * 0.14;
            let stem_w = s * 0.14;
            let top = c.y - s * 0.42;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(c.x - w / 2.0, top), Vec2::new(w, bar_h)),
                0.0,
                color,
            );
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(c.x - stem_w / 2.0, top),
                    Vec2::new(stem_w, s * 0.84),
                ),
                0.0,
                color,
            );
        }
        Tool::Sticky => {
            // Filled note with a folded corner.
            let half = s * 0.38;
            let fold = s * 0.18;
            let tl = Pos2::new(c.x - half, c.y - half);
            let tr = Pos2::new(c.x + half, c.y - half);
            let br_inner = Pos2::new(c.x + half - fold, c.y + half);
            let bl = Pos2::new(c.x - half, c.y + half);
            let br = Pos2::new(c.x + half, c.y + half);
            painter.add(egui::Shape::convex_polygon(
                vec![tl, tr, br, br_inner, bl],
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [br_inner, Pos2::new(c.x + half - fold, c.y + half - fold)],
                stroke,
            );
            let _ = br;
        }
        Tool::Frame => {
            // Four corner brackets.
            let half = s * 0.4;
            let arm = s * 0.22;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = Pos2::new(c.x + sx * half, c.y + sy * half);
                painter.line_segment([corner, Pos2::new(corner.x - sx * arm, corner.y)], stroke);
                painter.line_segment([corner, Pos2::new(corner.x, corner.y - sy * arm)], stroke);
            }
        }
        Tool::Image => {
            // Picture: frame + sun + mountain.
            let half = s * 0.4;
            let frame = Rect::from_center_size(c, Vec2::splat(half * 2.0));
            painter.rect_stroke(frame, 0.0, stroke, egui::StrokeKind::Middle);
            painter.circle_filled(
                Pos2::new(c.x - half * 0.4, c.y - half * 0.35),
                s * 0.07,
                color,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(frame.left() + 2.0, frame.bottom() - 2.0),
                    Pos2::new(c.x - half * 0.1, c.y + half * 0.1),
                    Pos2::new(c.x + half * 0.3, frame.bottom() - 2.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
}
