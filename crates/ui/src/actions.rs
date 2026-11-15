use gpui::{App, KeyBinding, actions};

actions!(
    dbased,
    [RunCurrent, RunAll, CancelExecution, NewTab, CloseActiveTab]
);

/// Key context of the workspace root.
pub const WORKSPACE_CONTEXT: &str = "Workspace";
/// Added to the workspace context while the active tab runs, so Esc cancels only then.
pub const RUNNING_CONTEXT: &str = "Running";

/// Default keymap (docs/PROJECT.md 9.4: VS Code-like; Cmd replaces Ctrl on macOS).
/// Must be called after `gpui_component::init`: the editor binds the same keys in its
/// own context, and later bindings win a tie.
pub fn bind_keys(cx: &mut App) {
    // @NOTE: the editor owns these keys in its "Input" context, so they are bound there too.
    let editor = Some("Input");
    let workspace = Some(WORKSPACE_CONTEXT);
    cx.bind_keys([
        KeyBinding::new(RUN_CURRENT_KEYS, RunCurrent, editor),
        KeyBinding::new(RUN_CURRENT_KEYS, RunCurrent, workspace),
        KeyBinding::new(RUN_ALL_KEYS, RunAll, editor),
        KeyBinding::new(RUN_ALL_KEYS, RunAll, workspace),
        KeyBinding::new("escape", CancelExecution, Some("Running > Input")),
        KeyBinding::new("escape", CancelExecution, Some(RUNNING_CONTEXT)),
        KeyBinding::new(NEW_TAB_KEYS, NewTab, workspace),
        KeyBinding::new(CLOSE_TAB_KEYS, CloseActiveTab, workspace),
    ]);
}

#[cfg(target_os = "macos")]
const RUN_CURRENT_KEYS: &str = "cmd-enter";
#[cfg(not(target_os = "macos"))]
const RUN_CURRENT_KEYS: &str = "ctrl-enter";
#[cfg(target_os = "macos")]
const RUN_ALL_KEYS: &str = "cmd-shift-enter";
#[cfg(not(target_os = "macos"))]
const RUN_ALL_KEYS: &str = "ctrl-shift-enter";
#[cfg(target_os = "macos")]
const NEW_TAB_KEYS: &str = "cmd-t";
#[cfg(not(target_os = "macos"))]
const NEW_TAB_KEYS: &str = "ctrl-t";
#[cfg(target_os = "macos")]
const CLOSE_TAB_KEYS: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_TAB_KEYS: &str = "ctrl-w";
