//! egui chrome (DESIGN D10): tool bar, HUD, and canvas overlays.
//!
//! Canvas text is drawn here as an egui overlay until parley-in-vello lands; it is drawn
//! above the vello scene, so heavily stacked boards may show text over later shapes.

pub use e6_editor::Tool;
use e6_scene::{Camera, Element, ElementId, ElementKind};

pub mod icons;

/// Commands from the right-hand panels (library + layers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    None,
    /// Recolor the selected sticky with the given palette color.
    SetStickyColor(u32),
    Select(ElementId),
    ToggleHidden(ElementId),
    Insert(LibraryEntry),
}

/// Reusable presets the library panel can stamp onto the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryEntry {
    Sticky,
    Card,
    Heading,
    Frame16x9,
    FlowTrio,
}

/// Kind display names for the layers list.
pub fn kind_label(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Rect => "Rect",
        ElementKind::Ellipse => "Ellipse",
        ElementKind::Stroke => "Stroke",
        ElementKind::Text => "Text",
        ElementKind::Sticky => "Sticky",
        ElementKind::Image => "Image",
        ElementKind::Frame => "Frame",
        ElementKind::Sector => "Sector",
        ElementKind::Connector => "Arrow",
    }
}

/// The library content: preset stamps to insert. Returns the picked entry, if any.
pub fn draw_library_content(ui: &mut egui::Ui) -> Option<PanelAction> {
    let mut picked = None;
    const ENTRIES: [(LibraryEntry, &str); 5] = [
        (LibraryEntry::Sticky, "Sticky note"),
        (LibraryEntry::Card, "Card"),
        (LibraryEntry::Heading, "Heading"),
        (LibraryEntry::Frame16x9, "Frame 16:9"),
        (LibraryEntry::FlowTrio, "Flow (A -> B)"),
    ];
    for (entry, label) in ENTRIES {
        if ui.button(label).clicked() {
            picked = Some(PanelAction::Insert(entry));
        }
    }
    picked
}

/// The layers content: recent elements, newest first (capped), with per-row selection
/// and visibility toggle. Returns the action, if any.
pub fn draw_layers_content(
    ui: &mut egui::Ui,
    elements: &[Element],
    selection: &e6_editor::Selection,
) -> Option<PanelAction> {
    let mut action = None;
    const MAX_ROWS: usize = 300;
    let shown = elements.len().min(MAX_ROWS);
    egui::ScrollArea::vertical().show(ui, |row_area| {
        for element in elements.iter().rev().take(shown) {
            let id = element.id;
            row_area.horizontal(|row| {
                let mut label = match element.text.as_str() {
                    "" => kind_label(element.kind).to_owned(),
                    text => format!("{} · {}", kind_label(element.kind), text),
                };
                if label.len() > 24 {
                    label.truncate(24);
                }
                if row
                    .selectable_label(selection.contains(id), label)
                    .clicked()
                {
                    action = Some(PanelAction::Select(id));
                }
                let toggle = if element.hidden { "Show" } else { "Hide" };
                if row.small_button(toggle).clicked() {
                    action = Some(PanelAction::ToggleHidden(id));
                }
            });
        }
        if elements.len() > MAX_ROWS {
            row_area.label(format!("… and {} more", elements.len() - MAX_ROWS));
        }
    });
    action
}

/// What the status HUD shows.
#[derive(Debug, Clone, PartialEq)]
pub struct HudState {
    /// Board title, shown in the top bar.
    pub title: String,
    /// Camera zoom as screen pixels per world unit.
    pub zoom: f32,
    pub visible_elements: usize,
    pub total_elements: usize,
    pub selected_elements: usize,
    /// Exponential moving average of frame time, milliseconds.
    pub frame_ms: f32,
    /// Transient user-facing message (share/sync/export failures, join progress);
    /// shown until `notice_until`, then cleared. `None` shows the plain status.
    pub notice: Option<String>,
    pub notice_until: Option<web_time::Instant>,
}

/// Button availability in the action bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudAvailability {
    pub can_undo: bool,
    pub can_redo: bool,
    pub has_selection: bool,
    pub can_group: bool,
    pub any_selected_locked: bool,
    pub sketch: bool,
    /// True only when this client created the room (owns the read token) and can
    /// therefore hand out view-only links.
    pub can_share_view: bool,
    /// Present needs at least one frame on the board.
    pub can_present: bool,
    /// Dot grid visible on the canvas.
    pub show_grid: bool,
}

/// A command the user pressed in the action bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HudCommand {
    None,
    ToggleGrid,
    Undo,
    Redo,
    Reset,
    /// Rotate the selection by the given delta (radians; negative = clockwise).
    Rotate(f32),
    ExportJson,
    BringToFront,
    SendToBack,
    Align(e6_editor::align::AlignMode),
    Group,
    Ungroup,
    ToggleLock,
    ToggleSketch,
    InsertImage,
    Present,
    ShareEdit,
    ShareView,
    ExportSvg,
    ExportPng,
}

/// One-line status summary. Shared by the HUD and tests so the wording has one home.
pub fn format_status(state: &HudState) -> String {
    format!(
        "zoom {:.0}% \u{b7} {}/{} visible \u{b7} {} selected \u{b7} {:.1} ms/frame",
        state.zoom * 100.0,
        state.visible_elements,
        state.total_elements,
        state.selected_elements,
        state.frame_ms
    )
}

/// Vertical tool bar on the left; returns the tool the user clicked, if any.
/// The tool bar, in display order. Digits 1–9 pick tools in this same order (see
/// `tool_for_digit`); each button shows its digit as hover text.
const TOOLS: [(Tool, &str); 9] = [
    (Tool::Select, "Select"),
    (Tool::Pan, "Pan"),
    (Tool::Pen, "Pen"),
    (Tool::Rect, "Rect"),
    (Tool::Ellipse, "Ellipse"),
    (Tool::Connector, "Arrow"),
    (Tool::Text, "Text"),
    (Tool::Sticky, "Sticky"),
    (Tool::Frame, "Frame"),
];

/// The tool bound to a digit key (1–9), matching the tool bar's order.
pub fn tool_for_digit(digit: u8) -> Option<Tool> {
    if digit == 0 {
        return None;
    }
    TOOLS.get(usize::from(digit) - 1).map(|(tool, _)| *tool)
}

/// The tool rail: one column of icon buttons on the left edge, Miro-style. Each
/// button shows its name and digit hotkey on hover; the active tool is filled.
pub fn draw_tool_bar(ui: &mut egui::Ui, active: Tool) -> Option<Tool> {
    let mut picked = None;
    egui::Panel::left("e6_tool_bar")
        .exact_size(52.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0xFF, 0xFF, 0xFF))
                .inner_margin(egui::Margin::symmetric(0, 8)),
        )
        .show(ui, |bar| {
            bar.vertical(|bar| {
                bar.add_space(4.0);
                for (index, (tool, label)) in TOOLS.iter().enumerate() {
                    let (rect, response) =
                        bar.allocate_exact_size(egui::Vec2::splat(38.0), egui::Sense::click());
                    let hotkey_label = format!("{label} — {}", index + 1);
                    // A custom-painted button has no label otherwise: give the
                    // accesskit tree (screen readers, kittest queries) a name.
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, hotkey_label.clone())
                    });
                    let is_active = active == *tool;
                    if is_active {
                        bar.painter().rect_filled(
                            rect,
                            6.0,
                            egui::Color32::from_rgb(226, 226, 240),
                        );
                    } else if response.hovered() {
                        bar.painter().rect_filled(
                            rect,
                            6.0,
                            egui::Color32::from_rgb(240, 240, 246),
                        );
                    }
                    let icon_color = if is_active {
                        egui::Color32::from_rgb(26, 26, 46)
                    } else {
                        egui::Color32::from_rgb(90, 90, 110)
                    };
                    icons::tool_icon(bar.painter(), rect, *tool, icon_color);
                    if response.clicked() {
                        picked = Some(*tool);
                    }
                    response.on_hover_text(hotkey_label);
                }
            });
        });
    picked
}

/// Draws the bottom status HUD with the action bar; returns the pressed command.
/// Call once per pass onto the root UI, before the canvas.
/// Stroke settings for the Pen tool, owned by the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenSettings {
    pub width: f32,
    pub color: u32,
}

impl Default for PenSettings {
    fn default() -> Self {
        Self {
            width: 2.5,
            color: 0xFF1A_1A2E,
        }
    }
}

const PEN_WIDTHS: [(&str, f32); 3] = [("Thin", 1.5), ("Normal", 3.0), ("Thick", 6.0)];
/// Miro-style sticky palette.
const STICKY_COLORS: [u32; 6] = [
    0xFFF7_E97B, // yellow (default)
    0xFFFF_B3BA, // pink
    0xFFB3_DEDD, // mint
    0xFFB3_C7FF, // periwinkle
    0xFFFF_D99B, // peach
    0xFFD9_CCFF, // lavender
];
const PEN_COLORS: [u32; 6] = [
    0xFF1A_1A2E, // ink (dark)
    0xFFD6_4E2C, // orange-red
    0xFF2E_7D4E, // green
    0xFF2E_5E8E, // blue
    0xFF8E_44AD, // purple
    0xFFB8_862D, // ochre
];

/// Which tab is active in the right drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawerTab {
    Library,
    Layers,
    Code,
}

/// The right drawer: one panel, three tabs (Library | Layers | mol). Returns the
/// active tab, the library/layers action, and the mol action.
pub fn draw_right_drawer(
    ui: &mut egui::Ui,
    tab: DrawerTab,
    elements: &[Element],
    selection: &e6_editor::Selection,
    code_panel: &mut CodePanelState,
    pen: Option<&mut PenSettings>,
    sticky_color: Option<u32>,
) -> (DrawerTab, Option<PanelAction>, CodeAction) {
    let mut tab = tab;
    let mut panel_action = None;
    // The sticky palette sits beside the tab content: its action travels on its
    // own channel so the tab match below cannot clobber it.
    let mut palette_action: Option<PanelAction> = None;
    let mut code_action = CodeAction::None;
    egui::Panel::right("e6_drawer")
        .exact_size(248.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0xFF, 0xFF, 0xFF))
                .inner_margin(egui::Margin::same(10)),
        )
        .show(ui, |panel| {
            panel.horizontal(|row| {
                for (candidate, label) in [
                    (DrawerTab::Library, "Library"),
                    (DrawerTab::Layers, "Layers"),
                    (DrawerTab::Code, "mol"),
                ] {
                    if row.selectable_label(tab == candidate, label).clicked() {
                        tab = candidate;
                    }
                }
            });
            panel.add_space(6.0);
            // Sticky palette: shown while a single sticky is selected.
            if let Some(current) = sticky_color {
                panel.heading("Sticky color");
                panel.horizontal(|row| {
                    for (swatch_index, color) in STICKY_COLORS.iter().enumerate() {
                        let selected = current == *color;
                        let (rect, response) =
                            row.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Sticky color {}", swatch_index + 1),
                            )
                        });
                        let rgb = color & 0x00FF_FFFF;
                        row.painter().rect_filled(
                            rect,
                            4.0,
                            egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
                        );
                        if selected {
                            row.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(26, 26, 46)),
                                egui::StrokeKind::Middle,
                            );
                        } else if response.hovered() {
                            row.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 120, 140)),
                                egui::StrokeKind::Middle,
                            );
                        }
                        if response.clicked() {
                            eprintln!("DEBUG sticky swatch {} clicked", swatch_index + 1);
                            palette_action = Some(PanelAction::SetStickyColor(*color));
                        }
                    }
                });
                panel.separator();
                panel.add_space(4.0);
            }
            // Contextual Pen options, above the tabs while the Pen tool is active.
            if let Some(pen) = pen {
                panel.heading("Pen");
                panel.horizontal(|row| {
                    for (label, width) in PEN_WIDTHS {
                        let selected = (pen.width - width).abs() < 0.01;
                        if row.selectable_label(selected, label).clicked() {
                            pen.width = width;
                        }
                    }
                });
                panel.add_space(4.0);
                panel.horizontal(|row| {
                    for (swatch_index, color) in PEN_COLORS.iter().enumerate() {
                        let selected = pen.color == *color;
                        let (rect, response) =
                            row.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Pen color {}", swatch_index + 1),
                            )
                        });
                        let rgb = color & 0x00FF_FFFF;
                        row.painter().rect_filled(
                            rect,
                            4.0,
                            egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
                        );
                        if selected {
                            row.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(26, 26, 46)),
                                egui::StrokeKind::Middle,
                            );
                        } else if response.hovered() {
                            row.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 120, 140)),
                                egui::StrokeKind::Middle,
                            );
                        }
                        if response.clicked() {
                            pen.color = *color;
                        }
                    }
                });
                panel.separator();
                panel.add_space(4.0);
            }
            match tab {
                DrawerTab::Library => {
                    panel_action = draw_library_content(panel);
                }
                DrawerTab::Layers => {
                    panel_action = draw_layers_content(panel, elements, selection);
                }
                DrawerTab::Code => {
                    code_action = draw_code_content(panel, code_panel);
                }
            }
        });
    if panel_action.is_none() {
        panel_action = palette_action;
    }
    (tab, panel_action, code_action)
}

/// The top action bar: edit tools, arrange menu, insert/share/export menus, and
/// the transient notice. Miro-style chrome; keyboard shortcuts are unchanged.
pub fn draw_top_bar(
    ui: &mut egui::Ui,
    state: &HudState,
    availability: &HudAvailability,
) -> HudCommand {
    let mut command = HudCommand::None;
    egui::Panel::top("e6_top_bar")
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0xFF, 0xFF, 0xFF))
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |bar| {
            bar.horizontal(|bar| {
                bar.strong(&state.title);
                bar.separator();
                if bar
                    .add_enabled(availability.can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    command = HudCommand::Undo;
                }
                if bar
                    .add_enabled(availability.can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    command = HudCommand::Redo;
                }
                bar.separator();

                use e6_editor::align::AlignMode;
                let count = state.selected_elements;
                bar.menu_button("Arrange", |menu| {
                    let align_entries = [
                        ("Align left", AlignMode::Left, 2),
                        ("Align centers", AlignMode::CenterH, 2),
                        ("Align right", AlignMode::Right, 2),
                        ("Align top", AlignMode::Top, 2),
                        ("Align middles", AlignMode::CenterV, 2),
                        ("Align bottom", AlignMode::Bottom, 2),
                        ("Distribute horizontally", AlignMode::DistributeH, 3),
                        ("Distribute vertically", AlignMode::DistributeV, 3),
                    ];
                    menu.set_min_width(180.0);
                    for (label, mode, min) in align_entries {
                        if menu
                            .add_enabled(count >= min, egui::Button::new(label))
                            .clicked()
                        {
                            command = HudCommand::Align(mode);
                        }
                    }
                    menu.separator();
                    if menu
                        .add_enabled(availability.can_group, egui::Button::new("Group"))
                        .clicked()
                    {
                        command = HudCommand::Group;
                    }
                    if menu
                        .add_enabled(availability.has_selection, egui::Button::new("Ungroup"))
                        .clicked()
                    {
                        command = HudCommand::Ungroup;
                    }
                    let lock_label = if availability.any_selected_locked {
                        "Unlock"
                    } else {
                        "Lock"
                    };
                    if menu
                        .add_enabled(availability.has_selection, egui::Button::new(lock_label))
                        .clicked()
                    {
                        command = HudCommand::ToggleLock;
                    }
                    menu.separator();
                    if menu
                        .add_enabled(
                            availability.has_selection,
                            egui::Button::new("Rotate 90° clockwise"),
                        )
                        .clicked()
                    {
                        command = HudCommand::Rotate(std::f32::consts::FRAC_PI_2);
                    }
                    if menu
                        .add_enabled(
                            availability.has_selection,
                            egui::Button::new("Rotate 90° counter-clockwise"),
                        )
                        .clicked()
                    {
                        command = HudCommand::Rotate(-std::f32::consts::FRAC_PI_2);
                    }
                    menu.separator();
                    if menu
                        .add_enabled(
                            availability.has_selection,
                            egui::Button::new("Bring to front"),
                        )
                        .clicked()
                    {
                        command = HudCommand::BringToFront;
                    }
                    if menu
                        .add_enabled(
                            availability.has_selection,
                            egui::Button::new("Send to back"),
                        )
                        .clicked()
                    {
                        command = HudCommand::SendToBack;
                    }
                });

                bar.menu_button("Insert", |menu| {
                    if menu.button("Image…").clicked() {
                        command = HudCommand::InsertImage;
                    }
                });

                bar.menu_button("Share", |menu| {
                    if menu.button("Share edit link").clicked() {
                        command = HudCommand::ShareEdit;
                    }
                    if menu
                        .add_enabled(
                            availability.can_share_view,
                            egui::Button::new("Share view link"),
                        )
                        .clicked()
                    {
                        command = HudCommand::ShareView;
                    }
                });

                bar.menu_button("Export", |menu| {
                    if menu.button("SVG (cmd/ctrl+E)").clicked() {
                        command = HudCommand::ExportSvg;
                    }
                    if menu.button("PNG (cmd/ctrl+shift+E)").clicked() {
                        command = HudCommand::ExportPng;
                    }
                    if menu.button("JSON").clicked() {
                        command = HudCommand::ExportJson;
                    }
                });

                let sketch_label = if availability.sketch {
                    "Sketch: on"
                } else {
                    "Sketch"
                };
                if bar.button(sketch_label).clicked() {
                    command = HudCommand::ToggleSketch;
                }
                if bar
                    .add_enabled(availability.can_present, egui::Button::new("Present"))
                    .on_disabled_hover_text("Draw a frame first — Present walks the frames")
                    .clicked()
                {
                    command = HudCommand::Present;
                }
                let grid_label = if availability.show_grid {
                    "Grid: on"
                } else {
                    "Grid"
                };
                if bar.button(grid_label).clicked() {
                    command = HudCommand::ToggleGrid;
                }
                if bar.button("Reset").clicked() {
                    command = HudCommand::Reset;
                }

                bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |bar| {
                    if let Some(notice) = &state.notice {
                        bar.colored_label(egui::Color32::from_rgb(150, 90, 10), notice.clone());
                    }
                });
            });
        });
    command
}

/// The slim bottom status bar: zoom, visibility, selection, frame time.
pub fn draw_status_bar(ui: &mut egui::Ui, state: &HudState) {
    egui::Panel::bottom("e6_status_bar").show(ui, |bar| {
        bar.horizontal(|bar| {
            bar.label(format_status(state));
        });
    });
}

/// Draws text payloads of text/sticky elements as an egui overlay over the canvas.
///
/// egui painter coordinates are in points while the camera works in physical pixels, so
/// both position and font size divide by `pixels_per_point`. Tiny text (< 4 px on screen)
/// is dropped — the first LOD tier, applied to text early.
pub fn draw_canvas_text(
    ui: &mut egui::Ui,
    camera: &Camera,
    elements: &[Element],
    pixels_per_point: f32,
) {
    for element in elements {
        let labelled = matches!(
            element.kind,
            ElementKind::Text | ElementKind::Sticky | ElementKind::Frame
        );
        if !labelled || element.text.is_empty() {
            continue;
        }
        let screen = camera.world_to_screen(element.bounds.min);
        let font_px = element.font_size * camera.zoom;
        if font_px < 4.0 {
            continue;
        }
        let ink = element.stroke_color;
        // Frame labels sit just above the top-left corner; content labels inside.
        let (pos, anchor) = if element.kind == ElementKind::Frame {
            (
                egui::Pos2::new(
                    screen.x / pixels_per_point,
                    screen.y / pixels_per_point - 6.0,
                ),
                egui::Align2::LEFT_BOTTOM,
            )
        } else {
            (
                egui::Pos2::new(
                    screen.x / pixels_per_point + 6.0,
                    screen.y / pixels_per_point + 4.0,
                ),
                egui::Align2::LEFT_TOP,
            )
        };
        ui.painter().text(
            pos,
            anchor,
            element.text.as_str(),
            egui::FontId::proportional(font_px / pixels_per_point),
            egui::Color32::from_rgba_unmultiplied(
                ((ink >> 16) & 0xFF) as u8,
                ((ink >> 8) & 0xFF) as u8,
                (ink & 0xFF) as u8,
                255,
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_hotkeys_follow_tool_bar_order() -> Result<(), Box<dyn std::error::Error>> {
        for (index, (tool, _)) in TOOLS.iter().enumerate() {
            let digit = u8::try_from(index + 1)?;
            assert_eq!(tool_for_digit(digit), Some(*tool), "digit {digit}");
        }
        assert_eq!(tool_for_digit(0), None);
        assert_eq!(tool_for_digit(10), None);
        Ok(())
    }

    #[test]
    fn status_line_mentions_zoom_visibility_selection_and_frametime() {
        let line = format_status(&HudState {
            title: "Untitled board".to_owned(),
            notice: None,
            notice_until: None,
            zoom: 1.5,
            visible_elements: 42,
            total_elements: 2500,
            selected_elements: 3,
            frame_ms: 4.2,
        });
        assert_eq!(
            line,
            "zoom 150% \u{b7} 42/2500 visible \u{b7} 3 selected \u{b7} 4.2 ms/frame"
        );
    }
}

/// State of the mol code panel (owned by the app so edits persist across frames).
#[derive(Debug, Clone, Default)]
pub struct CodePanelState {
    pub text: String,
    pub errors: Vec<String>,
}

/// Buttons pressed in the code panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeAction {
    None,
    Expand,
    ImportMermaid,
}

/// The mol code panel: source editor, Expand/Import buttons, located errors.
pub fn draw_code_content(ui: &mut egui::Ui, state: &mut CodePanelState) -> CodeAction {
    let mut action = CodeAction::None;
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |editor| {
            editor.add(
                egui::TextEdit::multiline(&mut state.text)
                    .desired_width(200.0)
                    .hint_text("mol source…")
                    .font(egui::FontId::monospace(12.0)),
            );
        });
    ui.horizontal(|row| {
        if row.button("Expand").clicked() {
            action = CodeAction::Expand;
        }
        if row.button("Import Mermaid").clicked() {
            action = CodeAction::ImportMermaid;
        }
    });
    for error in &state.errors {
        ui.label(egui::RichText::new(error).color(egui::Color32::from_rgb(255, 128, 128)));
    }
    action
}
