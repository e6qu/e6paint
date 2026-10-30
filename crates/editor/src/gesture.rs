//! The pointer-gesture reducer: a pure state machine from pointer events to
//! board commands.
//!
//! The shell (egui adapter) translates frame-level input into
//! [`PointerEvent`]s, feeds them to [`step`] with a read-only element list,
//! and applies the returned [`Command`]s to the document. All decision logic
//! — hit-testing, selection changes, snapping, draft lifecycles, slop
//! thresholds — lives here, free of egui and of document mutation, so
//! interaction behavior is table-driven and property testable.

use crate::{Selection, Tool, resize};
use e6_scene::{Aabb, Element, ElementId, ElementKind, Point, Size, geom};

/// Screen-pixel movement below which a press-release counts as a click.
pub const CLICK_SLOP_PX: f32 = 4.0;
/// Screen-pixel distance before a pen sample is recorded.
pub const PEN_SAMPLE_PX: f32 = 3.0;
/// Screen-pixel reach of the alignment-snap correction.
pub const SNAP_THRESHOLD_PX: f32 = 6.0;
/// Screen-pixel radius of a resize handle's grab zone.
pub const GRAB_PX: f32 = 10.0;
/// Ink color for strokes, shape outlines, and placed text.
pub const INK: u32 = 0xFF1D_1D2B;
/// Sticky note fill.
pub const STICKY_FILL: u32 = 0xFFF7_E97B;
/// Default world-unit sizes for placed text and sticky notes.
pub const TEXT_PLACE_SIZE: f32 = 240.0;
pub const STICKY_SIZE: f32 = 220.0;
/// Default world-unit width for placed frames.
pub const FRAME_SIZE: f32 = 640.0;

/// One pointer transition, already converted to world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerEvent {
    /// A press began at `at` (the true press origin, not a later position).
    Press { at: Point },
    /// The pointer moved while a press was held.
    Move { at: Point },
    /// The press ended; `at` is the release position.
    Release { at: Point },
    /// A rapid second press over the same spot.
    DoubleClick { at: Point },
}

/// Per-step read-only context: the viewport facts gestures need.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub zoom: f32,
    /// Shift is held: clicks add to / remove from the selection.
    pub shift: bool,
    /// The pen tool's ink color and stroke width.
    pub pen_color: u32,
    pub pen_width: f32,
}

/// In-progress gesture bookkeeping. Everything here is derivable from the
/// event history, which is what makes the reducer testable.
#[derive(Debug, Default)]
pub struct GestureState {
    pub tool: Tool,
    pub drag: Option<Drag>,
    pub resize: Option<ResizeGesture>,
    pub marquee: Option<Marquee>,
    pub draft: Option<Draft>,
    /// Press position of a place-tool gesture in flight.
    pub press: Option<Point>,
    /// World x positions of the active vertical alignment guides.
    pub guides_x: Vec<f32>,
    /// World y positions of the active horizontal alignment guides.
    pub guides_y: Vec<f32>,
}

/// An element move in progress.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    /// World position of the previous event; deltas come from this.
    pub last: Point,
}

/// A handle resize in progress.
#[derive(Debug, Clone, Copy)]
pub struct ResizeGesture {
    pub handle: resize::ResizeHandle,
    pub original: Aabb,
    pub rotation: f32,
    pub anchor: Point,
}

/// A rubber-band selection in progress.
#[derive(Debug, Clone, Copy)]
pub struct Marquee {
    pub start: Point,
    pub end: Point,
}

impl Marquee {
    pub fn bounds(&self) -> Aabb {
        Aabb::from_two_points(self.start, self.end)
    }
}

/// A creation gesture in progress.
#[derive(Debug, Clone)]
pub enum Draft {
    Pen {
        points: Vec<Point>,
        last: Point,
    },
    Shape {
        kind: ElementKind,
        start: Point,
        end: Point,
    },
    Connector {
        start: Point,
        end: Point,
    },
}

/// A pure mutation the shell applies to the document or selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Translate {
        id: ElementId,
        dx: f32,
        dy: f32,
    },
    SetGeometry {
        id: ElementId,
        bounds: Aabb,
    },
    InsertShape {
        kind: ElementKind,
        bounds: Aabb,
        fill: u32,
        stroke: u32,
        stroke_width: f32,
    },
    InsertStroke {
        bounds: Aabb,
        points: Vec<Point>,
        color: u32,
        width: f32,
    },
    InsertPlaced {
        kind: ElementKind,
        bounds: Aabb,
        fill: u32,
        font: f32,
        name: String,
        stroke: Option<(u32, f32)>,
    },
    Select(Vec<ElementId>),
    ToggleSelect(ElementId),
    ClearSelection,
    /// Select the document element the shell just inserted.
    SelectInserted,
    BeginEditing(ElementId),
    /// Open the inline editor on the just-inserted element.
    BeginEditingInserted,
    ReattachSelectionToFrames,
    Commit,
}

/// One reducer step: fold an event into the state and return the commands it
/// implies. `elements` is the read-only board; `selection` the current
/// selection, which this function never mutates directly — it returns
/// commands instead.
pub fn step(
    state: &mut GestureState,
    elements: &[Element],
    selection: &Selection,
    view: View,
    event: PointerEvent,
) -> Vec<Command> {
    match state.tool {
        Tool::Select => select_step(state, elements, selection, view, event),
        Tool::Pen => pen_step(state, view, event),
        Tool::Rect | Tool::Ellipse => shape_step(state, view, event),
        Tool::Connector => connector_step(state, view, event),
        Tool::Text | Tool::Sticky | Tool::Frame => {
            place_step(state, elements, selection, view, event)
        }
        Tool::Pan | Tool::Image => Vec::new(),
    }
}

/// Escape pressed: drop the in-flight draft or marquee; if neither is active,
/// clear the selection.
pub fn cancel(state: &mut GestureState) -> Vec<Command> {
    if state.draft.take().is_some() || state.marquee.take().is_some() {
        Vec::new()
    } else {
        vec![Command::ClearSelection]
    }
}

/// Whether a world point hits the topmost unlocked, visible element.
fn topmost_at(elements: &[Element], world: Point) -> Option<Element> {
    elements
        .iter()
        .rev()
        .find(|e| !e.locked && !e.hidden && e.bounds.contains(e.to_local(world)))
        .cloned()
}

/// Elements whose `frame` tag names `frame_id`, in paint order.
fn frame_members(elements: &[Element], frame_id: ElementId) -> Vec<ElementId> {
    let key = format!("el_{}", frame_id.raw());
    elements
        .iter()
        .filter(|e| e.frame.as_deref() == Some(key.as_str()))
        .map(|e| e.id)
        .collect()
}

/// Elements sharing `id`'s group tag (empty when `id` is ungrouped).
fn group_members(elements: &[Element], id: ElementId) -> Vec<ElementId> {
    let tag = elements
        .iter()
        .find(|e| e.id == id)
        .and_then(|e| e.group.clone());
    match tag {
        Some(key) => elements
            .iter()
            .filter(|e| e.group.as_deref() == Some(key.as_str()))
            .map(|e| e.id)
            .collect(),
        None => vec![id],
    }
}

/// Union bounds of the selected elements.
fn selection_bounds(elements: &[Element], selection: &Selection) -> Option<Aabb> {
    let mut bounds: Option<Aabb> = None;
    for element in elements {
        if selection.contains(element.id) {
            bounds = Some(match bounds {
                Some(b) => b.union(element.bounds),
                None => element.bounds,
            });
        }
    }
    bounds
}

/// Snap `delta` against the non-selected elements; returns the corrected delta
/// plus the guide positions that fired.
fn snapped_delta(
    elements: &[Element],
    selection: &Selection,
    moving: Aabb,
    dx: f32,
    dy: f32,
    zoom: f32,
) -> (f32, f32, Vec<f32>, Vec<f32>) {
    let candidates: Vec<Aabb> = elements
        .iter()
        .filter(|e| !selection.contains(e.id))
        .map(|e| e.bounds)
        .collect();
    let snap = crate::align::snap_delta(
        moving,
        &candidates,
        dx,
        dy,
        SNAP_THRESHOLD_PX / zoom.max(0.01),
    );
    (snap.dx, snap.dy, snap.guides_x, snap.guides_y)
}

fn select_step(
    state: &mut GestureState,
    elements: &[Element],
    selection: &Selection,
    view: View,
    event: PointerEvent,
) -> Vec<Command> {
    let mut commands = Vec::new();
    match event {
        PointerEvent::DoubleClick { at } => {
            match topmost_at(elements, at) {
                Some(element)
                    if matches!(element.kind, ElementKind::Text | ElementKind::Sticky) =>
                {
                    commands.push(Command::Select(vec![element.id]));
                    commands.push(Command::BeginEditing(element.id));
                }
                None => {
                    // Double-click empty canvas drops a sticky and opens it.
                    let bounds = Aabb::from_min_size(at, Size::new(STICKY_SIZE, STICKY_SIZE));
                    commands.push(Command::InsertPlaced {
                        kind: ElementKind::Sticky,
                        bounds,
                        fill: STICKY_FILL,
                        font: 24.0,
                        name: String::new(),
                        stroke: None,
                    });
                    commands.push(Command::Commit);
                    commands.push(Command::SelectInserted);
                    commands.push(Command::BeginEditingInserted);
                    state.tool = Tool::Select;
                }
                Some(_) => {
                    // A double-click over a plain element is an ordinary press.
                    commands.extend(select_press(state, elements, selection, view, at));
                }
            }
        }
        PointerEvent::Press { at } => {
            commands.extend(select_press(state, elements, selection, view, at));
        }
        PointerEvent::Move { at } => {
            if let Some(resize) = state.resize.as_ref() {
                let bounds = if resize.rotation == 0.0 {
                    resize::resize_bounds(resize.original, resize.handle, at)
                } else {
                    resize::resize_bounds_rotated(
                        resize.original,
                        resize.handle,
                        resize.rotation,
                        at,
                        resize.anchor,
                    )
                };
                if let Some(selected_id) = selection.iter().next() {
                    commands.push(Command::SetGeometry {
                        id: selected_id,
                        bounds,
                    });
                }
                return commands;
            }
            if let Some(drag) = state.drag.as_mut() {
                let dx = at.x - drag.last.x;
                let dy = at.y - drag.last.y;
                if let Some(moving) = selection_bounds(elements, selection) {
                    let (sdx, sdy, gx, gy) =
                        snapped_delta(elements, selection, moving, dx, dy, view.zoom);
                    state.guides_x = gx;
                    state.guides_y = gy;
                    let dx = dx + sdx;
                    let dy = dy + sdy;
                    for id in selection.iter() {
                        if elements.iter().any(|e| e.id == id && e.locked) {
                            continue;
                        }
                        commands.push(Command::Translate { id, dx, dy });
                    }
                }
                drag.last = at;
            } else if let Some(marquee) = state.marquee.as_mut() {
                marquee.end = at;
            }
        }
        PointerEvent::Release { at } => {
            state.guides_x.clear();
            state.guides_y.clear();
            // A stationary click arrives as a bare release (egui reports
            // clicked, never drag_started): select what is under the pointer.
            if state.drag.is_none() && state.resize.is_none() && state.marquee.is_none() {
                match topmost_at(elements, at) {
                    Some(element) => {
                        if view.shift {
                            commands.push(Command::ToggleSelect(element.id));
                        } else {
                            commands.push(Command::Select(vec![element.id]));
                        }
                    }
                    None => commands.push(Command::ClearSelection),
                }
                return commands;
            }
            if let Some(marquee) = state.marquee.take() {
                let region = marquee.bounds();
                let clicked_without_drag = region.size().width * view.zoom < CLICK_SLOP_PX
                    && region.size().height * view.zoom < CLICK_SLOP_PX;
                if !clicked_without_drag {
                    let hits: Vec<ElementId> = elements
                        .iter()
                        .filter(|e| !e.hidden && e.bounds.intersects(region))
                        .map(|e| e.id)
                        .collect();
                    commands.push(Command::Select(hits));
                }
            } else if state.drag.take().is_some() || state.resize.take().is_some() {
                commands.push(Command::ReattachSelectionToFrames);
                commands.push(Command::Commit);
            }
        }
    }
    commands
}

/// The press half of the select tool: resize handles first, then element
/// selection (shift toggles), then an empty-canvas marquee.
fn select_press(
    state: &mut GestureState,
    elements: &[Element],
    selection: &Selection,
    view: View,
    at: Point,
) -> Vec<Command> {
    let mut commands = Vec::new();
    if selection.len() == 1 {
        let grab = GRAB_PX / view.zoom.max(0.01);
        let selected = selection
            .iter()
            .next()
            .and_then(|id| elements.iter().find(|e| e.id == id));
        if let Some(element) = selected {
            let local = geom::unrotate_about(at, element.bounds.center(), element.rotation);
            if let Some(handle) = resize::handle_at(element.bounds, local, grab) {
                let anchor = geom::rotate_about(
                    resize::anchor_point(handle, element.bounds),
                    element.bounds.center(),
                    element.rotation,
                );
                state.resize = Some(ResizeGesture {
                    handle,
                    original: element.bounds,
                    rotation: element.rotation,
                    anchor,
                });
                return commands;
            }
        }
    }
    match topmost_at(elements, at) {
        Some(element) => {
            if view.shift {
                commands.push(Command::ToggleSelect(element.id));
            } else {
                if !selection.contains(element.id) {
                    let mut members = match element.kind {
                        ElementKind::Frame => frame_members(elements, element.id),
                        _ => group_members(elements, element.id),
                    };
                    members.push(element.id);
                    commands.push(Command::Select(members));
                }
                state.drag = Some(Drag { last: at });
            }
        }
        None => {
            commands.push(Command::ClearSelection);
            state.marquee = Some(Marquee { start: at, end: at });
        }
    }
    commands
}

fn pen_step(state: &mut GestureState, view: View, event: PointerEvent) -> Vec<Command> {
    let mut commands = Vec::new();
    match event {
        PointerEvent::Press { at } => {
            if state.draft.is_none() {
                state.draft = Some(Draft::Pen {
                    points: vec![at],
                    last: at,
                });
            }
        }
        PointerEvent::Move { at } => {
            if let Some(Draft::Pen { points, last }) = state.draft.as_mut() {
                let moved = (at.x - last.x).hypot(at.y - last.y) * view.zoom;
                if moved > PEN_SAMPLE_PX {
                    points.push(at);
                    *last = at;
                }
            }
        }
        PointerEvent::Release { .. } => {
            if let Some(Draft::Pen { points, .. }) = state.draft.take()
                && points.len() >= 2
                && let Some(bounds) = Aabb::from_points(&points)
            {
                commands.push(Command::InsertStroke {
                    bounds,
                    points,
                    color: view.pen_color,
                    width: view.pen_width,
                });
                commands.push(Command::Commit);
            }
        }
        PointerEvent::DoubleClick { .. } => {}
    }
    commands
}

fn shape_step(state: &mut GestureState, view: View, event: PointerEvent) -> Vec<Command> {
    let kind = match state.tool {
        Tool::Rect => ElementKind::Rect,
        Tool::Ellipse => ElementKind::Ellipse,
        _ => return Vec::new(),
    };
    let mut commands = Vec::new();
    match event {
        PointerEvent::Press { at } => {
            if state.draft.is_none() {
                state.draft = Some(Draft::Shape {
                    kind,
                    start: at,
                    end: at,
                });
            }
        }
        PointerEvent::Move { at } => {
            if let Some(Draft::Shape { end, .. }) = state.draft.as_mut() {
                *end = at;
            }
        }
        PointerEvent::Release { .. } => {
            if let Some(Draft::Shape { kind, start, end }) = state.draft.take()
                && (start.x - end.x).hypot(start.y - end.y) * view.zoom > CLICK_SLOP_PX
            {
                let bounds = Aabb::from_two_points(start, end);
                commands.push(Command::InsertShape {
                    kind,
                    bounds,
                    fill: 0x0000_0000,
                    stroke: INK,
                    stroke_width: 2.0,
                });
                commands.push(Command::Commit);
                commands.push(Command::SelectInserted);
                state.tool = Tool::Select;
            }
        }
        PointerEvent::DoubleClick { .. } => {}
    }
    commands
}

fn connector_step(state: &mut GestureState, view: View, event: PointerEvent) -> Vec<Command> {
    let mut commands = Vec::new();
    match event {
        PointerEvent::Press { at } => {
            if state.draft.is_none() {
                state.draft = Some(Draft::Connector { start: at, end: at });
            }
        }
        PointerEvent::Move { at } => {
            if let Some(Draft::Connector { end, .. }) = state.draft.as_mut() {
                *end = at;
            }
        }
        PointerEvent::Release { .. } => {
            if let Some(Draft::Connector { start, end }) = state.draft.take()
                && (start.x - end.x).hypot(start.y - end.y) * view.zoom > CLICK_SLOP_PX
            {
                let bounds = Aabb::from_two_points(start, end);
                commands.push(Command::InsertShape {
                    kind: ElementKind::Connector,
                    bounds,
                    fill: 0x0000_0000,
                    stroke: INK,
                    stroke_width: 2.0,
                });
                commands.push(Command::Commit);
                commands.push(Command::SelectInserted);
                state.tool = Tool::Select;
            }
        }
        PointerEvent::DoubleClick { .. } => {}
    }
    commands
}

fn place_step(
    state: &mut GestureState,
    elements: &[Element],
    selection: &Selection,
    view: View,
    event: PointerEvent,
) -> Vec<Command> {
    let _ = selection;
    let mut commands = Vec::new();
    // Placement happens on release over the same spot: drags are ignored, and
    // the press position is tracked so coalesced releases can tell the two
    // apart.
    let press = match event {
        PointerEvent::Press { at } => {
            state.press = Some(at);
            None
        }
        PointerEvent::Release { at } => {
            match state.press.take() {
                // A stationary click arrives as a bare release with no prior
                // Press event; the place position is the release position.
                None => Some(at),
                Some(press) => {
                    let moved = (press.x - at.x).hypot(press.y - at.y) * view.zoom;
                    (moved <= CLICK_SLOP_PX).then_some(at)
                }
            }
        }
        PointerEvent::DoubleClick { at } => {
            state.press = None;
            Some(at)
        }
        PointerEvent::Move { .. } => None,
    };
    let Some(at) = press else {
        return commands;
    };
    let (kind, size, fill, font) = match state.tool {
        Tool::Text => (ElementKind::Text, TEXT_PLACE_SIZE, INK, 28.0),
        Tool::Frame => (ElementKind::Frame, FRAME_SIZE, 0x0000_0000, 20.0),
        _ => (ElementKind::Sticky, STICKY_SIZE, STICKY_FILL, 24.0),
    };
    let short_side = match kind {
        ElementKind::Frame => size * 0.75,
        ElementKind::Sticky => size,
        _ => 48.0,
    };
    let frame_count = elements
        .iter()
        .filter(|e| e.kind == ElementKind::Frame)
        .count();
    let name = if kind == ElementKind::Frame {
        format!("Frame {}", frame_count + 1)
    } else {
        String::new()
    };
    let stroke = (kind == ElementKind::Frame).then_some((0xFF7C_7CE8, 2.0));
    let bounds = Aabb::from_min_size(at, Size::new(size, short_side));
    commands.push(Command::InsertPlaced {
        kind,
        bounds,
        fill,
        font,
        name,
        stroke,
    });
    commands.push(Command::Commit);
    commands.push(Command::SelectInserted);
    commands.push(Command::BeginEditingInserted);
    state.tool = Tool::Select;
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resize::ResizeHandle;

    fn rect(id: u64, x: f32, y: f32, w: f32, h: f32) -> Element {
        Element::new(
            ElementId::from_raw(id),
            ElementKind::Rect,
            Aabb::from_min_size(Point::new(x, y), Size::new(w, h)),
            0xFF00_00FF,
        )
    }

    fn view() -> View {
        View {
            zoom: 1.0,
            shift: false,
            pen_color: 0xFF00_00FF,
            pen_width: 2.0,
        }
    }

    fn at(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    fn press(x: f32, y: f32) -> PointerEvent {
        PointerEvent::Press { at: at(x, y) }
    }

    fn move_to(x: f32, y: f32) -> PointerEvent {
        PointerEvent::Move { at: at(x, y) }
    }

    fn release(x: f32, y: f32) -> PointerEvent {
        PointerEvent::Release { at: at(x, y) }
    }

    /// The test-side model: applies commands the way the production shell
    /// does — Select/Toggle/Clear update the selection, Translate/SetGeometry
    /// move the element, Insert is recorded.
    struct Harness {
        state: GestureState,
        elements: Vec<Element>,
        selection: Selection,
        commands: Vec<Command>,
    }

    impl Harness {
        fn new(tool: Tool, elements: Vec<Element>) -> Self {
            Self {
                state: GestureState {
                    tool,
                    ..GestureState::default()
                },
                elements,
                selection: Selection::new(),
                commands: Vec::new(),
            }
        }

        fn feed(&mut self, event: PointerEvent) {
            for command in step(
                &mut self.state,
                &self.elements,
                &self.selection,
                view(),
                event,
            ) {
                match command {
                    Command::Select(ids) => self.selection.set(ids),
                    Command::ToggleSelect(id) => self.selection.toggle(id),
                    Command::ClearSelection => self.selection.clear(),
                    Command::Translate { id, dx, dy } => {
                        if let Some(element) = self.elements.iter_mut().find(|e| e.id == id) {
                            element.bounds = Aabb::from_min_size(
                                Point::new(element.bounds.min.x + dx, element.bounds.min.y + dy),
                                element.bounds.size(),
                            );
                        }
                        self.commands.push(Command::Translate { id, dx, dy });
                    }
                    command => self.commands.push(command),
                }
            }
        }

        fn translates(&self) -> Vec<(f32, f32)> {
            self.commands
                .iter()
                .filter_map(|c| match c {
                    Command::Translate { dx, dy, .. } => Some((*dx, *dy)),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn press_move_release_translates_by_the_pointer_delta_exactly_once() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 100.0, 100.0, 50.0, 50.0)]);

        h.feed(press(110.0, 110.0));
        h.feed(move_to(120.0, 118.0));
        h.feed(move_to(160.0, 140.0));
        h.feed(release(160.0, 140.0));

        // The element moves by the pointer delta exactly once per movement —
        // never doubled, never accumulated from a stale origin.
        assert_eq!(h.translates(), vec![(10.0, 8.0), (40.0, 22.0)]);
        let element = &h.elements[0];
        assert_eq!(element.bounds.min, Point::new(150.0, 130.0));
    }

    /// Model property: however the driver chops a drag into events, the
    /// element ends exactly at press + pointer delta, and idle frames
    /// (repeated positions) contribute nothing.
    #[test]
    fn translate_total_equals_the_pointer_delta_for_any_event_chopping() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 0.0, 0.0, 80.0, 80.0)]);

        h.feed(press(10.0, 10.0));
        let path = [
            (25.0, 20.0),
            (40.0, 30.0),
            (55.0, 30.0),
            (55.0, 30.0), // idle frame: no movement
            (70.0, 60.0),
            (70.0, 60.0), // idle frame
        ];
        for at in path {
            h.feed(move_to(at.0, at.1));
        }
        h.feed(release(70.0, 60.0));

        let moving = h
            .translates()
            .into_iter()
            .filter(|(dx, dy)| *dx != 0.0 || *dy != 0.0)
            .count();
        assert_eq!(moving, 4, "idle frames contribute nothing");
        assert_eq!(h.elements[0].bounds.min, Point::new(60.0, 50.0));
    }

    #[test]
    fn drag_over_empty_canvas_selects_by_marquee() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 400.0, 400.0, 50.0, 50.0)]);

        h.feed(press(0.0, 0.0));
        h.feed(move_to(500.0, 500.0));
        h.feed(release(500.0, 500.0));

        assert!(h.selection.contains(ElementId::from_raw(1)));
    }

    #[test]
    fn a_stationary_release_over_empty_canvas_clears_but_selects_nothing() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 400.0, 400.0, 50.0, 50.0)]);

        h.feed(press(10.0, 10.0));
        assert!(h.selection.is_empty(), "empty press clears the selection");

        h.feed(release(10.0, 10.0));
        assert!(h.selection.is_empty());
    }

    #[test]
    fn snapping_never_adds_correction_without_candidates() {
        // With a single element on the board there are no candidates, so the
        // movement must be exactly the pointer delta — the regression that
        // doubled every drag.
        let mut h = Harness::new(Tool::Select, vec![rect(1, 100.0, 100.0, 50.0, 50.0)]);

        h.feed(press(110.0, 110.0));
        h.feed(move_to(150.0, 140.0));

        assert_eq!(h.translates(), vec![(40.0, 30.0)]);
    }

    #[test]
    fn snapping_pulls_the_delta_toward_a_candidates_edge_within_the_threshold() {
        // The moving element's right edge lands 3 world units left of the
        // candidate's left edge: the correction adds exactly those 3 units,
        // once.
        let mut h = Harness::new(
            Tool::Select,
            vec![
                rect(1, 100.0, 100.0, 50.0, 50.0),
                rect(2, 200.0, 100.0, 50.0, 50.0),
            ],
        );

        h.feed(press(110.0, 110.0));
        h.feed(move_to(157.0, 115.0));

        let moves = h.translates();
        assert_eq!(moves.len(), 1);
        let (dx, _) = moves[0];
        // Raw delta 47 leaves a 3-unit gap to the candidate's edge: the
        // correction closes it, once.
        assert!((dx - 50.0).abs() < 1e-4);
    }

    #[test]
    fn locked_selected_elements_do_not_translate() {
        let mut locked = rect(1, 100.0, 100.0, 50.0, 50.0);
        locked.locked = true;
        let mut h = Harness::new(Tool::Select, vec![locked]);
        h.selection.set([ElementId::from_raw(1)]);

        h.feed(press(110.0, 110.0));
        h.feed(move_to(150.0, 140.0));

        assert!(h.translates().is_empty());
    }

    #[test]
    fn resize_press_on_a_handle_then_move_emits_geometry() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 100.0, 100.0, 100.0, 100.0)]);
        h.selection.set([ElementId::from_raw(1)]);

        h.feed(press(200.0, 200.0));
        h.feed(move_to(260.0, 240.0));

        match h.commands.first() {
            Some(Command::SetGeometry { id, bounds }) => {
                assert_eq!(*id, ElementId::from_raw(1));
                assert_eq!(bounds.max, Point::new(260.0, 240.0));
                assert_eq!(bounds.min, Point::new(100.0, 100.0));
            }
            other => assert!(
                matches!(other, Some(Command::SetGeometry { .. })),
                "expected SetGeometry, got {other:?}"
            ),
        }
        let _ = ResizeHandle::SouthEast;
    }

    #[test]
    fn double_click_on_empty_canvas_places_a_sticky_and_opens_it() {
        let mut h = Harness::new(Tool::Select, Vec::new());

        h.feed(PointerEvent::DoubleClick {
            at: at(300.0, 300.0),
        });

        assert!(matches!(
            h.commands.first(),
            Some(Command::InsertPlaced {
                kind: ElementKind::Sticky,
                ..
            })
        ));
        assert!(h.commands.contains(&Command::SelectInserted));
        assert!(h.commands.contains(&Command::BeginEditingInserted));
        assert_eq!(h.state.tool, Tool::Select);
    }

    #[test]
    fn double_click_on_text_opens_the_editor_without_placing() {
        let mut text = rect(1, 100.0, 100.0, 80.0, 40.0);
        text.kind = ElementKind::Text;
        let mut h = Harness::new(Tool::Select, vec![text]);

        h.feed(PointerEvent::DoubleClick {
            at: at(120.0, 120.0),
        });

        assert!(
            h.commands
                .contains(&Command::BeginEditing(ElementId::from_raw(1)))
        );
        assert!(
            !h.commands
                .iter()
                .any(|c| matches!(c, Command::InsertPlaced { .. }))
        );
    }

    #[test]
    fn placing_a_frame_names_it_after_the_existing_frame_count() {
        let mut h = Harness::new(Tool::Frame, vec![rect(1, 0.0, 0.0, 40.0, 40.0)]);

        h.feed(press(300.0, 300.0));
        h.feed(release(300.0, 300.0));

        match h.commands.first() {
            Some(Command::InsertPlaced { name, bounds, .. }) => {
                assert_eq!(name, "Frame 1");
                assert_eq!(bounds.min, Point::new(300.0, 300.0));
            }
            other => assert!(
                matches!(other, Some(Command::InsertPlaced { .. })),
                "expected InsertPlaced, got {other:?}"
            ),
        }
    }

    #[test]
    fn a_place_drag_farther_than_the_slop_places_nothing() {
        let mut h = Harness::new(Tool::Sticky, Vec::new());

        h.feed(press(10.0, 10.0));
        h.feed(move_to(300.0, 300.0));
        h.feed(release(300.0, 300.0));

        assert!(
            h.commands.is_empty(),
            "drags with a place tool place nothing"
        );
    }

    #[test]
    fn a_bare_click_with_a_place_tool_places_at_the_pointer() {
        // egui reports a stationary click as clicked() only: the reducer sees
        // a bare release with no press event.
        let mut h = Harness::new(Tool::Sticky, Vec::new());

        h.feed(release(300.0, 300.0));

        assert!(matches!(
            h.commands.first(),
            Some(Command::InsertPlaced {
                kind: ElementKind::Sticky,
                ..
            })
        ));
    }

    #[test]
    fn a_bare_click_with_the_select_tool_selects_the_element_under_it() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 100.0, 100.0, 50.0, 50.0)]);

        h.feed(release(120.0, 120.0));

        assert!(h.selection.contains(ElementId::from_raw(1)));
    }

    #[test]
    fn a_bare_click_over_empty_canvas_clears_the_selection() {
        let mut h = Harness::new(Tool::Select, vec![rect(1, 100.0, 100.0, 50.0, 50.0)]);
        h.selection.set([ElementId::from_raw(1)]);

        h.feed(release(400.0, 400.0));

        assert!(h.selection.is_empty());
    }

    #[test]
    fn escape_drops_the_draft_and_then_clears_the_selection() {
        let mut h = Harness::new(Tool::Select, Vec::new());
        h.feed(press(10.0, 10.0));
        h.feed(move_to(40.0, 40.0));

        let commands = cancel(&mut h.state);
        assert!(commands.is_empty(), "the marquee is consumed first");

        let commands = cancel(&mut h.state);
        assert_eq!(commands, vec![Command::ClearSelection]);
    }
}
