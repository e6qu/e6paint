//! Headless interaction tests for the left tool rail.

use e6_ui::{DrawerTab, Tool, draw_right_drawer, draw_tool_bar};
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn rail_icons_are_labeled_and_pick_tools() {
    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut Option<Tool>| {
            let picked = draw_tool_bar(ui, Tool::Select);
            if let Some(tool) = picked {
                *state = Some(tool);
            }
        },
        None,
    );

    // Icons carry accessible labels (name + digit hotkey).
    harness.get_by_label("Pen — 3").click();
    harness.run();
    assert_eq!(*harness.state(), Some(Tool::Pen));

    harness.get_by_label("Frame — 9").click();
    harness.run();
    assert_eq!(*harness.state(), Some(Tool::Frame));
}

#[test]
fn rail_reports_no_pick_when_idle() {
    let mut harness = Harness::builder().allow_missing_glyphs().build_ui_state(
        |ui, state: &mut Option<Tool>| {
            *state = draw_tool_bar(ui, Tool::Select);
        },
        None,
    );
    harness.run();
    assert_eq!(*harness.state(), None);
    let _ = draw_right_drawer; // drawer coverage lives in top_bar_harness
    let _ = DrawerTab::Library;
}
