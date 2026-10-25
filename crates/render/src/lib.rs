//! Scene → Vello display list.
//!
//! Pure: `vello::Scene` is an encoded display list — no device, no queue, no IO here.
//! The GPU renderer lives in the app shell. Fragment caching, LOD tiers, and the SVG
//! writer are not implemented yet; the current path builds the display list directly.
//! Text glyphs are drawn as an egui overlay by the app for now; parley-in-vello lands
//! with the text polish work.

use std::sync::Arc;

pub mod lcg;

use vello::Scene;
use vello::kurbo::{
    Affine, BezPath, Circle, Point as KurboPoint, Rect, RoundedRect, Shape, Size as KurboSize,
    Stroke,
};
use vello::peniko::{self, Brush, Color, Fill};

use e6_scene::{Aabb, Camera, Element, ElementKind, Point, Size};

use lcg::Lcg;

/// Counters from one display-list build, for the HUD and perf tests.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DisplayStats {
    pub total: usize,
    pub visible: usize,
}

/// Level-of-detail tier for one element (DESIGN §6 tier ladder, v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod {
    /// Below ~2 screen px: invisible, don't encode anything.
    Skip,
    /// Below ~12 screen px: paint a plain filled placeholder instead of geometry.
    Placeholder,
    /// Full detail.
    Full,
}

fn lod_for(camera: &Camera, bounds: Aabb) -> Lod {
    let size = bounds.size();
    let screen_max = (size.width.max(size.height)) * camera.zoom;
    if screen_max < 2.0 {
        Lod::Skip
    } else if screen_max < 12.0 {
        Lod::Placeholder
    } else {
        Lod::Full
    }
}

/// Screen-space floor for stroke widths so hairlines stay visible when zoomed out.
const MIN_STROKE_PX: f64 = 1.0;

/// Sketch-mode jitter amplitude in screen pixels.
const SKETCH_JITTER_PX: f64 = 2.4;

/// Append the painted form of `elements` visible through `camera` to `scene`.
///
/// Screen space is the render target's *physical pixel* space, matching the camera.
/// Off-screen elements are skipped; that culling moves behind the spatial index when the
/// quadtree lands.
/// Miro-style dotted background: world-anchored dots that pan and zoom with the
/// board. Spacing doubles whenever dots would crowd below ~28 screen px, so
/// zooming out thins the grid instead of dense-cluttering it.
fn draw_dot_grid(scene: &mut Scene, camera: &Camera, visible_region: &Aabb) {
    let mut spacing = 32.0_f32;
    while spacing * camera.zoom < 28.0 {
        spacing *= 2.0;
    }
    // Screen-constant dot size.
    let radius = (1.3 / camera.zoom).max(0.05);
    let brush = brush_for(0xFFC9_C9D4);

    let start_x = (visible_region.min.x / spacing).floor() * spacing;
    let start_y = (visible_region.min.y / spacing).floor() * spacing;
    let cols = (((visible_region.max.x - start_x) / spacing).ceil() as usize).saturating_sub(1);
    let rows = (((visible_region.max.y - start_y) / spacing).ceil() as usize).saturating_sub(1);
    if cols.saturating_mul(rows) > 12_000 {
        return; // pathological zoom-out: skip rather than choke the frame
    }
    for row in 0..=rows {
        for col in 0..=cols {
            let world = Point::new(
                start_x + col as f32 * spacing,
                start_y + row as f32 * spacing,
            );
            let screen = screen_pos(camera, world);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &brush,
                None,
                &Circle::new((screen.x, screen.y), f64::from(radius)),
            );
        }
    }
}

pub fn build_display_list(
    scene: &mut Scene,
    camera: &Camera,
    viewport: Size,
    elements: &[Element],
    sketch: bool,
) -> DisplayStats {
    let visible_region = camera.viewport_aabb(viewport);
    draw_dot_grid(scene, camera, &visible_region);
    let mut stats = DisplayStats {
        total: elements.len(),
        visible: 0,
    };
    for element in elements {
        if element.hidden || !rotated_region_aabb(element).intersects(visible_region) {
            continue;
        }
        let lod = lod_for(camera, element.bounds);
        if lod == Lod::Skip {
            continue;
        }
        let brush = brush_for(element.color);
        let transform = element_transform(camera, element);
        if lod == Lod::Placeholder {
            let rect = screen_rect(camera, element.bounds);
            scene.fill(Fill::NonZero, transform, &brush, None, &rect);
            stats.visible += 1;
            continue;
        }
        let transform = element_transform(camera, element);
        match element.kind {
            ElementKind::Stroke => draw_stroke(scene, camera, element, transform),
            ElementKind::Connector => draw_connector(scene, camera, element, transform),
            ElementKind::Sector => {
                // Slice geometry arrives as a polygon in element points; fill it closed.
                let mut polygon: Vec<Point> = element.points.clone();
                if let Some(first) = polygon.first().copied() {
                    polygon.push(first);
                }
                let path = closed_path(polygon);
                scene.fill(Fill::NonZero, transform, &brush, None, &path);
            }
            ElementKind::Image => draw_image(scene, camera, element, transform),
            ElementKind::Sticky => {
                let rect = screen_rect(camera, element.bounds);
                let card = RoundedRect::from_rect(rect, 8.0);
                scene.fill(Fill::NonZero, transform, &brush, None, &card);
            }
            ElementKind::Ellipse => {
                let polygon = ellipse_polygon(camera, element.bounds);
                paint_sketchable(
                    scene,
                    camera,
                    element,
                    &brush,
                    polygon,
                    sketch,
                    element.id.raw(),
                );
            }
            // Text paints in the app's egui overlay, not the vello scene.
            ElementKind::Text => {}
            // Rect is also the placeholder shape for every kind without refined geometry yet.
            _ => {
                let rect = screen_rect(camera, element.bounds);
                let corners = vec![
                    Point::new(rect.x0 as f32, rect.y0 as f32),
                    Point::new(rect.x1 as f32, rect.y0 as f32),
                    Point::new(rect.x1 as f32, rect.y1 as f32),
                    Point::new(rect.x0 as f32, rect.y1 as f32),
                ];
                paint_sketchable(
                    scene,
                    camera,
                    element,
                    &brush,
                    corners,
                    sketch,
                    element.id.raw(),
                );
            }
        }
        stats.visible += 1;
    }
    stats
}

/// Stroke-or-fill one closed screen-space polygon, roughened in sketch mode.
fn paint_sketchable(
    scene: &mut Scene,
    camera: &Camera,
    element: &Element,
    brush: &Brush,
    polygon: Vec<Point>,
    sketch: bool,
    seed: u64,
) {
    let transform = element_transform(camera, element);
    let shape = if sketch {
        closed_path(rough_polygon(polygon, seed, SKETCH_JITTER_PX))
    } else {
        closed_path(polygon)
    };
    paint_outline_or_fill(scene, camera, element, brush, &shape, transform);
}

/// Draw a decoded image scaled into the element's screen rect.
fn draw_image(scene: &mut Scene, camera: &Camera, element: &Element, transform: Affine) {
    let Some(asset) = element.image.as_ref() else {
        return;
    };
    let rect = screen_rect(camera, element.bounds);
    if rect.width() < 0.5 || rect.height() < 0.5 || asset.width == 0 || asset.height == 0 {
        return;
    }
    let pixels: Arc<dyn AsRef<[u8]> + Send + Sync> = asset.rgba.clone();
    let data = peniko::Blob::new(pixels);
    let image_data = peniko::ImageData {
        data,
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: asset.width,
        height: asset.height,
    };
    let brush = peniko::ImageBrush::new(image_data);
    let local = Affine::translate((rect.x0, rect.y0))
        * Affine::scale_non_uniform(
            rect.width() / f64::from(asset.width),
            rect.height() / f64::from(asset.height),
        );
    scene.draw_image(&brush, transform * local);
}

/// Polygon sampled around the ellipse inscribed in `bounds` (screen space).
fn ellipse_polygon(camera: &Camera, bounds: Aabb) -> Vec<Point> {
    let rect = screen_rect(camera, bounds);
    let cx = (rect.x0 + rect.x1) * 0.5;
    let cy = (rect.y0 + rect.y1) * 0.5;
    let rx = rect.width() * 0.5;
    let ry = rect.height() * 0.5;
    const STEPS: usize = 20;
    (0..STEPS)
        .map(|step| {
            let angle = core::f64::consts::TAU * (step as f64) / (STEPS as f64);
            Point::new(
                (cx + rx * angle.cos()) as f32,
                (cy + ry * angle.sin()) as f32,
            )
        })
        .collect()
}

/// Closed [`BezPath`] through `points`.
fn closed_path(points: Vec<Point>) -> BezPath {
    let mut path = BezPath::new();
    for (index, point) in points.iter().enumerate() {
        let kurbo = KurboPoint::new(f64::from(point.x), f64::from(point.y));
        if index == 0 {
            path.move_to(kurbo);
        } else {
            path.line_to(kurbo);
        }
    }
    path.close_path();
    path
}

/// Subdivide every edge and jitter interior vertices perpendicular to it — the
/// hand-drawn look. Deterministic per seed; corners stay anchored.
fn rough_polygon(points: Vec<Point>, seed: u64, amplitude: f64) -> Vec<Point> {
    let count = points.len();
    if count < 2 {
        return points;
    }
    let mut out: Vec<Point> = Vec::new();
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        out.push(a);
        let (dx, dy) = (f64::from(b.x - a.x), f64::from(b.y - a.y));
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1.0 {
            continue;
        }
        let steps = (length / 26.0).ceil().max(1.0) as usize;
        let mut rng = Lcg::from_seed(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (nx, ny) = (-dy / length, dx / length);
        for step in 1..steps {
            let t = step as f64 / steps as f64;
            let offset = (rng.next_unit() - 0.5) * 2.0 * amplitude;
            out.push(Point::new(
                (f64::from(a.x) + dx * t + nx * offset) as f32,
                (f64::from(a.y) + dy * t + ny * offset) as f32,
            ));
        }
    }
    out
}

fn paint_outline_or_fill(
    scene: &mut Scene,
    camera: &Camera,
    element: &Element,
    brush: &Brush,
    shape: &impl Shape,
    transform: Affine,
) {
    if element.stroke_width > 0.0 {
        let style = Stroke::new(screen_width(camera, element.stroke_width));
        let outline = brush_for(element.stroke_color);
        scene.stroke(&style, transform, &outline, None, shape);
    } else {
        scene.fill(Fill::NonZero, transform, brush, None, shape);
    }
}

fn draw_stroke(scene: &mut Scene, camera: &Camera, element: &Element, transform: Affine) {
    let mut path = BezPath::new();
    for (index, point) in element.points.iter().enumerate() {
        let screen = camera.world_to_screen(*point);
        let kurbo = KurboPoint::new(f64::from(screen.x), f64::from(screen.y));
        if index == 0 {
            path.move_to(kurbo);
        } else {
            path.line_to(kurbo);
        }
    }
    let width = if element.stroke_width > 0.0 {
        element.stroke_width
    } else {
        2.0
    };
    let style = Stroke::new(screen_width(camera, width).max(MIN_STROKE_PX));
    let outline = brush_for(element.stroke_color);
    scene.stroke(&style, transform, &outline, None, &path);
}

fn draw_connector(scene: &mut Scene, camera: &Camera, element: &Element, transform: Affine) {
    let (Some(start), Some(end)) = (element.points.first(), element.points.get(1)) else {
        return;
    };
    let a = camera.world_to_screen(*start);
    let b = camera.world_to_screen(*end);
    let mut path = BezPath::new();
    path.move_to(KurboPoint::new(f64::from(a.x), f64::from(a.y)));
    path.line_to(KurboPoint::new(f64::from(b.x), f64::from(b.y)));
    let width = if element.stroke_width > 0.0 {
        element.stroke_width
    } else {
        2.0
    };
    let style = Stroke::new(screen_width(camera, width).max(MIN_STROKE_PX));
    let outline = brush_for(element.stroke_color);
    scene.stroke(&style, transform, &outline, None, &path);

    // Arrowhead at the end: a filled triangle aligned with the line direction.
    let (dx, dy) = (f64::from(b.x - a.x), f64::from(b.y - a.y));
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    let head = (screen_width(camera, 24.0) * 2.0).max(10.0);
    let half = head * 0.4;
    let tip = KurboPoint::new(f64::from(b.x), f64::from(b.y));
    let base = KurboPoint::new(f64::from(b.x) - ux * head, f64::from(b.y) - uy * head);
    let mut head_path = BezPath::new();
    head_path.move_to(tip);
    head_path.line_to(KurboPoint::new(base.x - uy * half, base.y + ux * half));
    head_path.line_to(KurboPoint::new(base.x + uy * half, base.y - ux * half));
    head_path.close_path();
    scene.fill(Fill::NonZero, transform, &outline, None, &head_path);
}

/// Map a world point to canvas-local screen pixels (vello draws in that space).
fn screen_pos(camera: &Camera, world: Point) -> KurboPoint {
    let screen = camera.world_to_screen(world);
    KurboPoint::new(f64::from(screen.x), f64::from(screen.y))
}

/// Transform for one element: rotation about its screen-space bounds center.
/// Identity for unrotated elements.
fn element_transform(camera: &Camera, element: &Element) -> Affine {
    if element.rotation == 0.0 {
        return Affine::IDENTITY;
    }
    let center = element.bounds.center();
    let center_screen = camera.world_to_screen(center);
    Affine::rotate_about(
        f64::from(element.rotation),
        KurboPoint::new(f64::from(center_screen.x), f64::from(center_screen.y)),
    )
}

/// World-space AABB of the rotated bounds (for culling and hover).
fn rotated_region_aabb(element: &Element) -> Aabb {
    if element.rotation == 0.0 {
        return element.bounds;
    }
    let center = element.bounds.center();
    let angle = f64::from(element.rotation);
    let (sin, cos) = angle.sin_cos();
    let half_w = f64::from(element.bounds.size().width) / 2.0;
    let half_h = f64::from(element.bounds.size().height) / 2.0;
    // Corner offsets rotated about the center, in world units.
    let corners = [
        (-half_w, -half_h),
        (half_w, -half_h),
        (-half_w, half_h),
        (half_w, half_h),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (dx, dy) in corners {
        let rx = dx * cos - dy * sin;
        let ry = dx * sin + dy * cos;
        min_x = min_x.min(f64::from(center.x) + rx);
        min_y = min_y.min(f64::from(center.y) + ry);
        max_x = max_x.max(f64::from(center.x) + rx);
        max_y = max_y.max(f64::from(center.y) + ry);
    }
    Aabb::from_min_size(
        Point::new(min_x as f32, min_y as f32),
        Size::new((max_x - min_x) as f32, (max_y - min_y) as f32),
    )
}

fn screen_rect(camera: &Camera, bounds: Aabb) -> Rect {
    let min = camera.world_to_screen(bounds.min);
    let size = bounds.size();
    Rect::from_origin_size(
        KurboPoint::new(f64::from(min.x), f64::from(min.y)),
        KurboSize::new(
            f64::from(size.width * camera.zoom),
            f64::from(size.height * camera.zoom),
        ),
    )
}

fn screen_width(camera: &Camera, world_width: f32) -> f64 {
    f64::from(world_width * camera.zoom)
}

fn brush_for(color: u32) -> Brush {
    Color::from_rgba8(
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
        255,
    )
    .into()
}

/// XML-escape attribute/text content.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn svg_color(color: u32) -> String {
    format!("#{:06x}", color & 0xFF_FFFF)
}

/// SVG text export: `<text>` elements at element positions (no glyph outlines).
fn svg_text_lines(element: &Element) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, line) in element.text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let x = f64::from(element.bounds.min.x + 6.0);
        let y = f64::from(element.bounds.min.y + 20.0 + index as f32 * element.font_size * 1.3);
        lines.push(format!(
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{}\">{}</text>",
            f64::from(element.font_size),
            svg_color(element.stroke_color),
            xml_escape(line),
        ));
    }
    lines
}

/// Export the board (world-space elements, full detail) as a standalone SVG document
/// sized to the content bounds. Vector-faithful to the display list: rects, ellipses,
/// polylines, sectors, connectors; text as real `<text>` runs. Hidden elements skip;
/// locked ones still export (visibility is a view concern, lock is an edit concern).
pub fn export_svg(elements: &[Element], title: &str) -> String {
    let Some(first) = elements.first() else {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"0\" height=\"0\"/>".into();
    };
    let mut bounds = first.bounds;
    for element in elements {
        if let Some(point_bounds) = Aabb::from_points(&element.points) {
            bounds = bounds.union(point_bounds);
        }
        bounds = bounds.union(element.bounds);
    }
    let pad = 40.0;
    let view = Aabb::from_min_size(
        Point::new(bounds.min.x - pad, bounds.min.y - pad),
        Size::new(
            bounds.size().width + pad * 2.0,
            bounds.size().height + pad * 2.0,
        ),
    );
    svg_document(elements, title, view)
}

/// Emit the board inside an explicit world-space viewBox (crop window for the
/// software renderer).
/// `M x y L x y …` for a point list; `None` when fewer than two points can form
/// a path. The single builder for every SVG polyline (strokes, connectors, sectors).
fn svg_polyline_path(points: &[Point]) -> Option<String> {
    if points.len() < 2 {
        return None;
    }
    let mut path = String::new();
    for (index, point) in points.iter().enumerate() {
        let command = if index == 0 { "M" } else { "L" };
        path.push_str(&format!(
            "{command} {} {}",
            f64::from(point.x),
            f64::from(point.y)
        ));
    }
    Some(path)
}

fn svg_document(elements: &[Element], title: &str, view: Aabb) -> String {
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">\n<title>{}</title>\n",
        f64::from(view.size().width),
        f64::from(view.size().height),
        f64::from(view.min.x),
        f64::from(view.min.y),
        f64::from(view.size().width),
        f64::from(view.size().height),
        xml_escape(title),
    );

    for element in elements {
        if element.hidden {
            continue;
        }
        let fill = svg_color(element.color);
        let stroke = svg_color(element.stroke_color);
        let stroke_width = f64::from(element.stroke_width.max(2.0));
        match element.kind {
            ElementKind::Rect => out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                f64::from(element.bounds.min.x),
                f64::from(element.bounds.min.y),
                f64::from(element.bounds.size().width),
                f64::from(element.bounds.size().height),
                fill, stroke, stroke_width
            )),
            ElementKind::Ellipse => {
                let center = element.bounds.center();
                out.push_str(&format!(
                    "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                    f64::from(center.x),
                    f64::from(center.y),
                    f64::from(element.bounds.size().width * 0.5),
                    f64::from(element.bounds.size().height * 0.5),
                    fill, stroke, stroke_width
                ));
            }
            ElementKind::Sticky => out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"{}\"/>\n",
                f64::from(element.bounds.min.x),
                f64::from(element.bounds.min.y),
                f64::from(element.bounds.size().width),
                f64::from(element.bounds.size().height),
                fill
            )),
            ElementKind::Stroke | ElementKind::Connector => {
                let Some(path) = svg_polyline_path(&element.points) else {
                    continue;
                };
                out.push_str(&format!(
                    "<path d=\"{path}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{stroke_width}\"/>\n"
                ));
            }
            ElementKind::Sector => {
                let Some(mut path) = svg_polyline_path(&element.points) else {
                    continue;
                };
                path.push_str(" Z");
                out.push_str(&format!(
                    "<path d=\"{path}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1\"/>\n"
                ));
            }
            ElementKind::Image => out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ddd\" stroke=\"{stroke}\"/>\n",
                f64::from(element.bounds.min.x),
                f64::from(element.bounds.min.y),
                f64::from(element.bounds.size().width),
                f64::from(element.bounds.size().height),
            )),
            ElementKind::Frame => out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\"/>\n",
                f64::from(element.bounds.min.x),
                f64::from(element.bounds.min.y),
                f64::from(element.bounds.size().width),
                f64::from(element.bounds.size().height),
                stroke
            )),
            ElementKind::Text => {
                for line in svg_text_lines(element) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }

    out.push_str("</svg>");
    out
}

/// Rasterized RGBA pixels (straight alpha) of a rendered view.
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// CPU-render the visible world region into RGBA pixels — the view-only fallback for
/// environments without WebGPU, and the same path PNG export uses. Deterministic.
pub fn rasterize_view(
    elements: &[Element],
    camera: &Camera,
    viewport: Size,
    _title: &str,
) -> Result<Raster, String> {
    // The view overlay carries no title; the SVG title element is omitted here.
    let view = svg_document(elements, "e6paint view", camera.viewport_aabb(viewport));
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&view, &options).map_err(|err| err.to_string())?;
    let width = (viewport.width.max(1.0)) as u32;
    let height = (viewport.height.max(1.0)) as u32;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("pixmap allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    // resvg pixels are premultiplied; consumers (egui) want straight alpha.
    let mut rgba = pixmap.take();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = pixel[3];
        if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = u8::try_from((u16::from(*channel) * 255) / u16::from(alpha).max(1))
                    .unwrap_or(255);
            }
        }
    }
    Ok(Raster {
        width: width as usize,
        height: height as usize,
        rgba,
    })
}

#[cfg(test)]
mod raster_tests {
    use super::*;
    use e6_scene::{ElementId, ElementKind, Point, Size};

    #[test]
    fn raster_view_renders_opaque_pixels_over_the_viewport()
    -> Result<(), Box<dyn std::error::Error>> {
        let rect = Element::new(
            ElementId::from_raw(1),
            ElementKind::Rect,
            Aabb::from_min_size(Point::new(0.0, 0.0), Size::new(400.0, 300.0)),
            0xFFFF_0000,
        );
        let camera = Camera::default();
        let raster = rasterize_view(&[rect], &camera, Size::new(200.0, 150.0), "t")
            .map_err(|err| err.to_string())?;
        assert_eq!(raster.width, 200);
        assert_eq!(raster.height, 150);
        assert!(
            raster.rgba.as_chunks::<4>().0.contains(&[255, 0, 0, 255]),
            "red rect pixels are opaque in the raster"
        );
        Ok(())
    }
}

#[cfg(test)]
mod svg_tests {
    use super::*;
    use e6_scene::{Aabb, ElementId, ElementKind, Point, Size};

    fn element(id: u64, kind: ElementKind) -> Element {
        Element::new(
            ElementId::from_raw(id),
            kind,
            Aabb::from_min_size(Point::new(10.0, 10.0), Size::new(50.0, 40.0)),
            0xFF22_4488,
        )
    }

    #[test]
    fn svg_export_shapes_labels_and_bounds() {
        let elements = vec![element(1, ElementKind::Rect), {
            let mut text = element(2, ElementKind::Text);
            text.text = "hello <board>".into();
            text.font_size = 24.0;
            text
        }];
        let svg = export_svg(&elements, "Test board");
        assert!(svg.starts_with("<svg"), "svg root");
        assert!(svg.contains("<title>Test board</title>"));
        assert!(svg.contains("<rect"));
        assert!(svg.contains("hello &lt;board&gt;"), "text is XML-escaped");
    }

    #[test]
    fn hidden_elements_skip_export() {
        let mut hidden = element(1, ElementKind::Rect);
        hidden.hidden = true;
        let svg = export_svg(&[hidden], "t");
        assert!(!svg.contains("<rect"), "hidden rect must not export");
    }

    #[test]
    fn png_export_rasterizes_to_png_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let elements = vec![element(1, ElementKind::Rect)];
        let png = export_png(&elements, "t", 2)?;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG magic bytes");
        Ok(())
    }

    #[test]
    fn connectors_export_as_paths() {
        let mut connector = element(1, ElementKind::Connector);
        connector.points = vec![Point::new(0.0, 0.0), Point::new(80.0, 60.0)];
        connector.stroke_width = 2.0;
        let svg = export_svg(&[connector], "t");
        assert!(svg.contains("<path"), "connector renders as a path");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use e6_scene::{ElementId, Point};

    fn element(id: u64, x: f32, y: f32, kind: ElementKind) -> Element {
        Element::new(
            ElementId::from_raw(id),
            kind,
            Aabb::from_min_size(Point::new(x, y), Size::new(50.0, 40.0)),
            0xFF22_4488,
        )
    }

    #[test]
    fn culls_offscreen_elements_and_counts_visibility() {
        let camera = Camera::default();
        let viewport = Size::new(100.0, 100.0);
        let elements = [
            element(1, 10.0, 10.0, ElementKind::Rect),
            element(2, 500.0, 500.0, ElementKind::Rect),
            element(3, 80.0, 80.0, ElementKind::Ellipse),
        ];

        let mut scene = Scene::new();
        let stats = build_display_list(&mut scene, &camera, viewport, &elements, false);

        assert_eq!(stats.total, 3);
        assert_eq!(stats.visible, 2);
    }

    #[test]
    fn zooming_out_reveals_more_elements() {
        let mut camera = Camera::default();
        let elements = [element(1, 400.0, 400.0, ElementKind::Rect)];

        let mut scene = Scene::new();
        let zoomed_in = build_display_list(
            &mut scene,
            &camera,
            Size::new(100.0, 100.0),
            &elements,
            false,
        );
        camera.zoom_at(Point::ZERO, 0.05);
        let zoomed_out = build_display_list(
            &mut scene,
            &camera,
            Size::new(100.0, 100.0),
            &elements,
            false,
        );

        assert_eq!(zoomed_in.visible, 0);
        assert_eq!(zoomed_out.visible, 1);
    }

    #[test]
    fn strokes_and_connectors_paint_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
        let camera = Camera::default();
        let mut stroke = element(1, 0.0, 0.0, ElementKind::Stroke).outlined(0xFF00_00FF, 3.0);
        stroke.bounds = Aabb::from_points(&[Point::new(0.0, 0.0), Point::new(80.0, 60.0)])
            .ok_or("two points have bounds")?;
        stroke.points = vec![
            Point::new(0.0, 0.0),
            Point::new(40.0, 30.0),
            Point::new(80.0, 60.0),
        ];
        let mut connector = element(2, 0.0, 0.0, ElementKind::Connector).outlined(0xFF00_00FF, 2.0);
        connector.bounds = stroke.bounds;
        connector.points = vec![Point::new(0.0, 0.0), Point::new(80.0, 60.0)];
        let sticky = element(3, 10.0, 10.0, ElementKind::Sticky);

        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &camera,
            Size::new(400.0, 400.0),
            &[stroke, connector, sticky],
            false,
        );
        assert_eq!(stats.visible, 3);
        Ok(())
    }

    #[test]
    fn sketch_roughening_is_deterministic_and_subdivides() {
        let square = vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        ];
        let rough = rough_polygon(square.clone(), 42, 2.4);
        assert_eq!(
            rough,
            rough_polygon(square.clone(), 42, 2.4),
            "same seed, same output"
        );
        assert!(rough.len() > square.len(), "edges are subdivided");
        assert_eq!(rough[0], square[0], "corners stay anchored");
    }

    #[test]
    fn sketch_mode_builds_a_display_list() {
        let camera = Camera::default();
        let elements = [element(1, 10.0, 10.0, ElementKind::Rect)];
        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &camera,
            Size::new(400.0, 400.0),
            &elements,
            true,
        );
        assert_eq!(stats.visible, 1);
    }

    #[test]
    fn image_elements_paint_via_draw_image() -> Result<(), Box<dyn std::error::Error>> {
        let camera = Camera::default();
        let mut image = element(1, 10.0, 10.0, ElementKind::Image);
        image.bounds = Aabb::from_min_size(Point::new(10.0, 10.0), Size::new(40.0, 30.0));
        image.image = Some(e6_scene::ImageAsset {
            width: 4,
            height: 3,
            rgba: std::sync::Arc::new(vec![255; 4 * 3 * 4]),
        });

        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &camera,
            Size::new(400.0, 400.0),
            &[image],
            false,
        );
        assert_eq!(stats.visible, 1);
        Ok(())
    }

    #[test]
    fn image_elements_without_payload_are_skipped() {
        let camera = Camera::default();
        let image = element(1, 10.0, 10.0, ElementKind::Image);
        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &camera,
            Size::new(400.0, 400.0),
            &[image],
            false,
        );
        assert_eq!(stats.visible, 1, "counted, but drawing early-returns");
    }

    #[test]
    fn degenerate_connectors_are_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let camera = Camera::default();
        let mut connector = element(1, 0.0, 0.0, ElementKind::Connector).outlined(0xFF00_00FF, 2.0);
        connector.points = vec![Point::new(5.0, 5.0)];
        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &camera,
            Size::new(100.0, 100.0),
            &[connector],
            false,
        );
        assert_eq!(stats.visible, 1, "counted, but drawing early-returns");
        Ok(())
    }
}

/// PNG export: rasterize the exported SVG at `scale` (1 = world units -> px) via
/// `resvg`. Returns encoded PNG bytes.
pub fn export_png(elements: &[Element], title: &str, scale: u32) -> Result<Vec<u8>, String> {
    let svg = export_svg(elements, title);
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|err| err.to_string())?;
    let size = tree.size();
    let scale_f = scale as f32;
    let width = ((size.width() * scale_f) as u32).max(1);
    let height = ((size.height() * scale_f) as u32).max(1);
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("pixmap allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale_f, scale_f),
        &mut pixmap.as_mut(),
    );

    pixmap.encode_png().map_err(|err| err.to_string())
}
