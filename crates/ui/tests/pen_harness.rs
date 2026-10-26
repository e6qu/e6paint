//! Headless tests for the contextual Pen options in the drawer.

use e6_ui::{CodePanelState, DrawerTab, PenSettings, draw_right_drawer};
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn pen_options_dispatch_width_and_color() {
    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut PenSettings| {
            let mut pen = PenSettings {
                width: state.width,
                color: state.color,
            };
            let (_tab, _action, _code) = draw_right_drawer(
                ui,
                DrawerTab::Library,
                &[],
                &e6_editor::Selection::new(),
                &mut CodePanelState::default(),
                Some(&mut pen),
                None,
            );
            *state = pen;
        },
        PenSettings::default(),
    );

    harness.get_by_label("Thick").click();
    harness.run();
    assert!((harness.state().width - 6.0).abs() < 0.01);

    harness.get_by_label("Pen color 4").click();
    harness.run();
    assert_eq!(harness.state().color & 0x00FF_FFFF, 0x2E_5E8E);
}

#[test]
fn pen_options_absent_without_pen_tool() {
    // With pen = None the drawer renders only the tabs and their content.
    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut bool| {
            let (_tab, _action, _code) = draw_right_drawer(
                ui,
                DrawerTab::Library,
                &[],
                &e6_editor::Selection::new(),
                &mut CodePanelState::default(),
                None,
                None,
            );
            *state = true;
        },
        false,
    );
    harness.run();
    assert!(*harness.state());
}
