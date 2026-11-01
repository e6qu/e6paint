//! Keyboard shortcut mapping: key + modifiers + UI context → action.
//!
//! Pure and table-tested. The shell translates egui key state into
//! [`ShortcutKey`]/[`ShortcutMods`] and executes the returned
//! [`ShortcutAction`] with its own document and platform calls.

use crate::Tool;

/// The keys the board listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
    Escape,
    Delete,
    Backspace,
    R,
    D,
    E,
    Z,
    Y,
    Enter,
    Digit(u8),
    A,
    G,
    BracketRight,
    BracketLeft,
    C,
    X,
    V,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

/// Modifier state for the pressed key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShortcutMods {
    pub command: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl ShortcutMods {
    fn command_or_ctrl(&self) -> bool {
        self.command || self.ctrl
    }
}

/// UI facts that gate shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShortcutContext {
    /// The inline text editor is open: it owns the keyboard.
    pub editing: bool,
    /// A text field (inline editor or the mol code panel) owns the keyboard.
    pub keyboard_busy: bool,
    pub has_selection: bool,
    /// The board clipboard holds something to paste.
    pub clipboard_nonempty: bool,
}

/// A keyboard-triggered board action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShortcutAction {
    ExpandMol,
    /// Escape: drop the in-flight draft or marquee; the shell clears the
    /// selection when nothing was consumed.
    CancelGesture,
    DeleteSelection,
    SelectAll,
    Group,
    Ungroup,
    BringToFront,
    SendToBack,
    RotateSelection,
    DuplicateSelection,
    CopySelection,
    CutSelection,
    Paste,
    NudgeSelection {
        dx: f32,
        dy: f32,
    },
    ExportSvg,
    ExportPng,
    Undo,
    Redo,
    ApplyTool(Tool),
}

/// Map a pressed key to its action, honoring the same precedence and guards
/// as the interactive shell: Escape defers to an open editor; a focused text
/// field suppresses everything except command/ctrl+Enter; board actions
/// requiring a selection are skipped without one.
pub fn shortcut_action(
    key: ShortcutKey,
    mods: ShortcutMods,
    ctx: ShortcutContext,
) -> Option<ShortcutAction> {
    // Escape while editing belongs to the inline editor (it applies the
    // buffer and closes there, so queued keystrokes survive).
    if key == ShortcutKey::Escape {
        if ctx.editing {
            return None;
        }
        return Some(ShortcutAction::CancelGesture);
    }

    // Command/ctrl+Enter expands the mol panel — the standard "render the
    // source" gesture. It works even while typing in the panel.
    if key == ShortcutKey::Enter && mods.command_or_ctrl() {
        return Some(ShortcutAction::ExpandMol);
    }

    // Everything below acts on the board: a focused text field owns the
    // keyboard, so its Delete, undo/redo, and digits must edit text, never
    // the board.
    if ctx.keyboard_busy {
        return None;
    }
    let step = if mods.shift { 10.0 } else { 1.0 };

    match key {
        ShortcutKey::Delete | ShortcutKey::Backspace if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::DeleteSelection)
        }
        ShortcutKey::R if ctx.has_selection => Some(ShortcutAction::RotateSelection),
        ShortcutKey::D if mods.command_or_ctrl() && ctx.has_selection => {
            Some(ShortcutAction::DuplicateSelection)
        }
        ShortcutKey::C if mods.command_or_ctrl() && !ctx.editing && ctx.has_selection => {
            Some(ShortcutAction::CopySelection)
        }
        ShortcutKey::X if mods.command_or_ctrl() && !ctx.editing && ctx.has_selection => {
            Some(ShortcutAction::CutSelection)
        }
        ShortcutKey::V if mods.command_or_ctrl() && !ctx.editing && ctx.clipboard_nonempty => {
            Some(ShortcutAction::Paste)
        }
        ShortcutKey::E if mods.command_or_ctrl() => {
            if mods.shift {
                Some(ShortcutAction::ExportPng)
            } else {
                Some(ShortcutAction::ExportSvg)
            }
        }
        ShortcutKey::Z if mods.command_or_ctrl() => {
            if mods.shift {
                Some(ShortcutAction::Redo)
            } else {
                Some(ShortcutAction::Undo)
            }
        }
        ShortcutKey::Y if mods.command_or_ctrl() => Some(ShortcutAction::Redo),
        ShortcutKey::Digit(n) if !ctx.editing => {
            let tool = match n {
                1 => Tool::Select,
                2 => Tool::Pan,
                3 => Tool::Pen,
                4 => Tool::Rect,
                5 => Tool::Ellipse,
                6 => Tool::Connector,
                7 => Tool::Text,
                8 => Tool::Sticky,
                9 => Tool::Frame,
                _ => return None,
            };
            Some(ShortcutAction::ApplyTool(tool))
        }
        ShortcutKey::A if mods.command_or_ctrl() && !ctx.editing => Some(ShortcutAction::SelectAll),
        ShortcutKey::G if mods.command_or_ctrl() && mods.shift && ctx.has_selection => {
            Some(ShortcutAction::Ungroup)
        }
        ShortcutKey::G if mods.command_or_ctrl() && ctx.has_selection => {
            Some(ShortcutAction::Group)
        }
        ShortcutKey::BracketRight if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::BringToFront)
        }
        ShortcutKey::BracketLeft if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::SendToBack)
        }
        ShortcutKey::ArrowLeft if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::NudgeSelection { dx: -step, dy: 0.0 })
        }
        ShortcutKey::ArrowRight if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::NudgeSelection { dx: step, dy: 0.0 })
        }
        ShortcutKey::ArrowUp if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::NudgeSelection { dx: 0.0, dy: -step })
        }
        ShortcutKey::ArrowDown if ctx.has_selection && !ctx.editing => {
            Some(ShortcutAction::NudgeSelection { dx: 0.0, dy: step })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(command: bool, shift: bool) -> ShortcutMods {
        ShortcutMods {
            command,
            ctrl: false,
            shift,
        }
    }

    fn ctx(editing: bool, busy: bool, has_selection: bool) -> ShortcutContext {
        ShortcutContext {
            editing,
            keyboard_busy: busy,
            has_selection,
            clipboard_nonempty: true,
        }
    }

    #[test]
    fn select_all_group_and_z_order_map_with_guards() {
        // cmd+A selects all — even on an empty board (harmless) — but never
        // while a text editor owns cmd+A for its own select-all.
        assert_eq!(
            shortcut_action(ShortcutKey::A, mods(true, false), ctx(false, false, false)),
            Some(ShortcutAction::SelectAll)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::A, mods(true, false), ctx(true, false, true)),
            None,
            "editing: cmd+A selects the text"
        );
        // Group needs 2+ selected (shell validates the count); ungroup needs
        // cmd+shift+G.
        assert_eq!(
            shortcut_action(ShortcutKey::G, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::Group)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::G, mods(true, true), ctx(false, false, true)),
            Some(ShortcutAction::Ungroup)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::G, mods(true, false), ctx(false, false, false)),
            None
        );
        // Z-order brackets: bare keys, selection required.
        assert_eq!(
            shortcut_action(
                ShortcutKey::BracketRight,
                mods(false, false),
                ctx(false, false, true)
            ),
            Some(ShortcutAction::BringToFront)
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::BracketLeft,
                mods(false, false),
                ctx(false, false, true)
            ),
            Some(ShortcutAction::SendToBack)
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::BracketRight,
                mods(false, false),
                ctx(false, false, false)
            ),
            None
        );
    }

    #[test]
    fn arrows_nudge_the_selection_and_shift_steps_ten_pixels() {
        assert_eq!(
            shortcut_action(
                ShortcutKey::ArrowLeft,
                mods(false, false),
                ctx(false, false, true)
            ),
            Some(ShortcutAction::NudgeSelection { dx: -1.0, dy: 0.0 })
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::ArrowDown,
                mods(false, true),
                ctx(false, false, true)
            ),
            Some(ShortcutAction::NudgeSelection { dx: 0.0, dy: 10.0 })
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::ArrowUp,
                mods(false, false),
                ctx(false, false, false)
            ),
            None,
            "no selection: nothing to nudge"
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::ArrowUp,
                mods(false, false),
                ctx(true, false, true)
            ),
            None,
            "editing: arrows belong to the text caret"
        );
    }

    #[test]
    fn clipboard_shortcuts_require_selection_and_a_nonempty_clipboard() {
        assert_eq!(
            shortcut_action(ShortcutKey::D, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::DuplicateSelection)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::C, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::CopySelection)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::X, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::CutSelection)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::V, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::Paste)
        );
        // No selection: copy and cut do nothing...
        assert_eq!(
            shortcut_action(ShortcutKey::C, mods(true, false), ctx(false, false, false)),
            None
        );
        // ...but paste works regardless, when the clipboard holds something.
        assert_eq!(
            shortcut_action(ShortcutKey::V, mods(true, false), ctx(false, false, false)),
            Some(ShortcutAction::Paste)
        );
        // Empty clipboard: paste is inert.
        let mut empty = ctx(false, false, true);
        empty.clipboard_nonempty = false;
        assert_eq!(
            shortcut_action(ShortcutKey::V, mods(true, false), empty),
            None
        );
        // Editing suppresses all three.
        for key in [ShortcutKey::C, ShortcutKey::X, ShortcutKey::V] {
            assert_eq!(
                shortcut_action(key, mods(true, false), ctx(true, false, true)),
                None,
                "{key:?} fired while editing"
            );
        }
    }

    #[test]
    fn escape_defers_to_the_editor_but_cancels_when_free() {
        assert_eq!(
            shortcut_action(
                ShortcutKey::Escape,
                mods(false, false),
                ctx(true, false, true)
            ),
            None,
            "editing: the inline editor owns Escape"
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::Escape,
                mods(false, false),
                ctx(false, false, false)
            ),
            Some(ShortcutAction::CancelGesture)
        );
    }

    #[test]
    fn command_enter_expands_even_while_typing() {
        assert_eq!(
            shortcut_action(
                ShortcutKey::Enter,
                mods(true, false),
                ctx(true, true, false)
            ),
            Some(ShortcutAction::ExpandMol)
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::Enter,
                mods(false, false),
                ctx(false, false, true)
            ),
            None,
            "bare Enter is not a shortcut"
        );
    }

    #[test]
    fn a_busy_keyboard_suppresses_board_actions() {
        for key in [
            ShortcutKey::Delete,
            ShortcutKey::Backspace,
            ShortcutKey::R,
            ShortcutKey::D,
            ShortcutKey::E,
            ShortcutKey::Z,
            ShortcutKey::Y,
            ShortcutKey::Digit(4),
        ] {
            assert_eq!(
                shortcut_action(key, mods(true, true), ctx(false, true, true)),
                None,
                "{key:?} fired while a text field owned the keyboard"
            );
        }
    }

    #[test]
    fn board_actions_require_a_selection() {
        assert_eq!(
            shortcut_action(
                ShortcutKey::Delete,
                mods(false, false),
                ctx(false, false, false)
            ),
            None
        );
        assert_eq!(
            shortcut_action(ShortcutKey::R, mods(false, false), ctx(false, false, true)),
            Some(ShortcutAction::RotateSelection)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::D, mods(true, false), ctx(false, false, true)),
            Some(ShortcutAction::DuplicateSelection)
        );
    }

    #[test]
    fn undo_redo_follows_the_modifier_alphabet() {
        assert_eq!(
            shortcut_action(ShortcutKey::Z, mods(true, false), ctx(false, false, false)),
            Some(ShortcutAction::Undo)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::Z, mods(true, true), ctx(false, false, false)),
            Some(ShortcutAction::Redo)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::Y, mods(true, false), ctx(false, false, false)),
            Some(ShortcutAction::Redo)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::Z, mods(false, false), ctx(false, false, false)),
            None,
            "bare Z is not undo"
        );
    }

    #[test]
    fn export_picks_png_with_shift() {
        assert_eq!(
            shortcut_action(ShortcutKey::E, mods(true, false), ctx(false, false, false)),
            Some(ShortcutAction::ExportSvg)
        );
        assert_eq!(
            shortcut_action(ShortcutKey::E, mods(true, true), ctx(false, false, false)),
            Some(ShortcutAction::ExportPng)
        );
    }

    #[test]
    fn digits_map_to_toolbar_order() {
        assert_eq!(
            shortcut_action(
                ShortcutKey::Digit(1),
                mods(false, false),
                ctx(false, false, false)
            ),
            Some(ShortcutAction::ApplyTool(Tool::Select))
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::Digit(9),
                mods(false, false),
                ctx(false, false, false)
            ),
            Some(ShortcutAction::ApplyTool(Tool::Frame))
        );
        assert_eq!(
            shortcut_action(
                ShortcutKey::Digit(4),
                mods(false, false),
                ctx(true, false, false)
            ),
            None,
            "digits must not switch tools mid-edit"
        );
    }
}
