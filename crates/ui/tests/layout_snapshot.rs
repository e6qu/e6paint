//! Visual layout capture: renders the full chrome composition to a PNG so the
//! layout can be reviewed (and diffed) without a browser. Not an assertion
//! test beyond "it renders" - the PNG is the artifact.

use e6_ui::{
    CodePanelState, DrawerTab, HudAvailability, HudCommand, HudState, draw_right_drawer,
    draw_status_bar, draw_tool_bar, draw_top_bar,
};
use egui_kittest::{Harness, SnapshotOptions};

fn composition(ui: &mut egui::Ui, state: &mut (HudState, DrawerTab)) {
    static LIGHT: std::sync::Once = std::sync::Once::new();
    LIGHT.call_once(|| ui.ctx().set_theme(egui::Theme::Light));
    let (hud, drawer_tab) = state;
    let _ = draw_tool_bar(ui, e6_ui::Tool::Select);
    let availability = HudAvailability {
        can_undo: true,
        can_redo: true,
        has_selection: true,
        can_group: true,
        any_selected_locked: false,
        sketch: false,
        can_share_view: true,
        can_present: true,
        show_grid: true,
    };
    let command = draw_top_bar(ui, hud, &availability);
    let _ = command;
    let _ = HudCommand::None;

    let elements: Vec<e6_scene::Element> = Vec::new();
    let selection = e6_editor::Selection::new();
    let mut code_panel = CodePanelState {
        text: "flow \"Demo\" {\n  a -> b\n}".to_owned(),
        errors: Vec::new(),
    };
    let (new_tab, _panel_action, _code_action) = draw_right_drawer(
        ui,
        *drawer_tab,
        &elements,
        &selection,
        &mut code_panel,
        None,
        None,
    );
    *drawer_tab = new_tab;

    // Canvas stand-in: a bordered empty area where the board renders.
    let available = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(available, 0.0, egui::Color32::from_rgb(0xF5, 0xF5, 0xF7));
    draw_status_bar(ui, hud);
}

// Local visual-review tool, not a CI gate: platform rendering differs, and the
// snapshot must be regenerated (UPDATE_SNAPSHOTS=1) after intentional UI
// changes. Run with: cargo test -p e6-ui --test layout_snapshot -- --ignored
#[test]
#[ignore = "visual review tool; regenerate the PNG locally with UPDATE_SNAPSHOTS=1"]
fn render_layout_snapshot() {
    let hud = HudState {
        title: "Untitled board".to_owned(),
        zoom: 1.0,
        visible_elements: 0,
        total_elements: 0,
        selected_elements: 0,
        frame_ms: 0.0,
        notice: None,
        notice_until: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(composition, (hud, DrawerTab::Library));

    let options =
        SnapshotOptions::default().output_path("../../target/layout-snapshots/current.png");
    harness.snapshot_options("layout", &options);
    harness.run();
}
