//! Headless interaction tests for the top action bar (egui_kittest drives the
//! real egui UI through the accesskit tree).

use e6_scene::{Aabb, Element, ElementId, ElementKind, Point, Size};
use e6_ui::{DrawerTab, HudAvailability, HudCommand, HudState, draw_right_drawer, draw_top_bar};
use egui_kittest::{Harness, kittest::Queryable};

fn hud_state() -> HudState {
    HudState {
        title: "Untitled board".to_owned(),
        zoom: 1.0,
        visible_elements: 3,
        total_elements: 3,
        selected_elements: 1,
        frame_ms: 0.0,
        notice: None,
        notice_until: None,
    }
}

fn availability(can_undo: bool) -> HudAvailability {
    HudAvailability {
        can_undo,
        can_redo: can_undo,
        has_selection: true,
        can_group: true,
        any_selected_locked: false,
        sketch: false,
        can_share_view: true,
        can_present: true,
        show_grid: true,
    }
}

fn one_element() -> Vec<Element> {
    vec![Element::new(
        ElementId::from_raw(1),
        ElementKind::Rect,
        Aabb::from_min_size(Point::new(0.0, 0.0), Size::new(50.0, 40.0)),
        0xFF22_4488,
    )]
}

#[test]
fn undo_and_redo_buttons_dispatch_commands() {
    let state = HudCommand::None;
    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state| {
            let command = draw_top_bar(ui, &hud_state(), &availability(true));
            // egui runs multiple passes per harness step; keep the click's
            // command instead of letting an idle pass overwrite it.
            if command != HudCommand::None {
                *state = command;
            }
        },
        state,
    );

    // The board name is rendered as a labeled node in the top bar.
    harness.get_by_label("Untitled board").click();
    harness.run();

    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::Undo);

    harness.get_by_label("Redo").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::Redo);
}

#[test]
fn undo_is_disabled_when_nothing_to_undo() {
    let state = HudCommand::None;
    let mut harness = Harness::new_ui_state(
        |ui, state| {
            *state = draw_top_bar(ui, &hud_state(), &availability(false));
        },
        state,
    );

    // A disabled button does not dispatch.
    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::None);
}

#[test]
fn arrange_menu_dispatches_group() {
    let state = HudCommand::None;
    let mut harness = Harness::new_ui_state(
        |ui, state| {
            let command = draw_top_bar(ui, &hud_state(), &availability(true));
            // egui runs multiple passes per harness step; keep the click's
            // command instead of letting an idle pass overwrite it.
            if command != HudCommand::None {
                *state = command;
            }
        },
        state,
    );

    harness.get_by_label("Arrange").click();
    harness.run();
    harness.get_by_label("Group").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::Group);
}

#[test]
fn share_menu_gates_view_link_on_ownership() {
    // With the read token: both share entries are enabled.
    let mut harness = Harness::new_ui_state(
        |ui, state: &mut HudCommand| {
            let command = draw_top_bar(ui, &hud_state(), &availability(true));
            // egui runs multiple passes per harness step; keep the click's
            // command instead of letting an idle pass overwrite it.
            if command != HudCommand::None {
                *state = command;
            }
        },
        HudCommand::None,
    );
    harness.get_by_label("Share").click();
    harness.run();
    harness.get_by_label("Share edit link").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::ShareEdit);
}

#[test]
fn export_menu_dispatches_svg() {
    let state = HudCommand::None;
    let mut harness = Harness::new_ui_state(
        |ui, state| {
            let command = draw_top_bar(ui, &hud_state(), &availability(true));
            // egui runs multiple passes per harness step; keep the click's
            // command instead of letting an idle pass overwrite it.
            if command != HudCommand::None {
                *state = command;
            }
        },
        state,
    );

    harness.get_by_label("Export").click();
    harness.run();
    harness.get_by_label("SVG (cmd/ctrl+E)").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::ExportSvg);
}

#[test]
fn present_and_reset_dispatch() {
    let state = HudCommand::None;
    let mut harness = Harness::new_ui_state(
        |ui, state| {
            let command = draw_top_bar(ui, &hud_state(), &availability(true));
            // egui runs multiple passes per harness step; keep the click's
            // command instead of letting an idle pass overwrite it.
            if command != HudCommand::None {
                *state = command;
            }
        },
        state,
    );

    harness.get_by_label("Present").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::Present);

    harness.get_by_label("Reset").click();
    harness.run();
    assert_eq!(*harness.state(), HudCommand::Reset);
}

#[test]
fn drawer_tabs_switch_content_and_dispatch_library_action() {
    let elements = one_element();
    let selection = e6_editor::Selection::new();
    let code_panel = std::cell::RefCell::new(e6_ui::CodePanelState::default());
    let action: std::cell::RefCell<Option<e6_ui::PanelAction>> = std::cell::RefCell::new(None);

    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut DrawerTab| {
            let elements = elements.clone();
            let (new_tab, panel_action, _code) = draw_right_drawer(
                ui,
                *state,
                &elements,
                &selection,
                &mut code_panel.borrow_mut(),
                None,
            );
            *state = new_tab;
            // Multi-pass: keep the action instead of letting an idle pass clear it.
            if let Some(insert_action) = panel_action {
                *action.borrow_mut() = Some(insert_action);
            }
        },
        DrawerTab::Library,
    );

    // Library tab shows the presets; clicking one dispatches Insert.
    harness.get_by_label("Sticky note").click();
    harness.run();
    assert!(matches!(
        *action.borrow(),
        Some(e6_ui::PanelAction::Insert(_))
    ));

    // Switch to the mol tab: the Expand button appears.
    harness.get_by_label("mol").click();
    harness.run();
    harness.get_by_label("Expand").click();
    harness.run();
    assert!(matches!(*harness.state(), DrawerTab::Code));
}
