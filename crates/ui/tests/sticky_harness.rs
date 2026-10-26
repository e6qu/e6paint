//! Headless tests for the sticky palette in the drawer.

use e6_ui::{CodePanelState, DrawerTab, PanelAction, draw_right_drawer};
use egui_kittest::{Harness, kittest::Queryable};
use std::cell::RefCell;

#[test]
fn sticky_palette_dispatches_recolor() {
    let current: RefCell<Option<u32>> = RefCell::new(Some(0xFFF7_E97B)); // yellow
    let picked: RefCell<Option<PanelAction>> = RefCell::new(None);

    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut DrawerTab| {
            let current = current.borrow().as_ref().copied();
            let (_tab, panel_action, _code) = draw_right_drawer(
                ui,
                *state,
                &[],
                &e6_editor::Selection::new(),
                &mut CodePanelState::default(),
                None,
                current,
            );
            if let Some(e6_ui::PanelAction::SetStickyColor(color)) = panel_action {
                *picked.borrow_mut() = Some(e6_ui::PanelAction::SetStickyColor(color));
            }
        },
        DrawerTab::Library,
    );

    harness.get_by_label("Sticky color 3").click();
    harness.run();
    let picked = *picked.borrow();
    assert!(
        matches!(picked, Some(PanelAction::SetStickyColor(c)) if c == 0xFFB3_DEDD),
        "mint swatch dispatches recolor"
    );
}
