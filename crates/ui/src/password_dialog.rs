use std::cell::Cell;

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use secrecy::SecretString;

use crate::{
    client::{self, CommandSink},
    state::UiState,
};

const DIALOG_WIDTH: f32 = 400.0;

/// Asks for a connection's password for this session (docs/PROJECT.md 5.1), with an
/// optional "save it" choice.
pub struct PasswordDialog {
    state: Entity<UiState>,
    sink: CommandSink,
    connection_name: String,
    password: Entity<InputState>,
    save: bool,
}

pub fn open(
    state: Entity<UiState>,
    sink: CommandSink,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<PasswordDialog>> {
    let connection = state.read(cx).flow.password_prompt.as_ref()?.connection;
    let connection_name = state.read(cx).connections.name(connection);
    let dialog = cx.new(|cx| PasswordDialog::new(state, sink, connection_name, window, cx));
    let content = dialog.clone();
    let on_ok = dialog.clone();
    let on_cancel = dialog.clone();
    let initial_focus = Cell::new(true);
    window.open_dialog(cx, move |shell, window, cx| {
        if initial_focus.replace(false) {
            content.update(cx, |dialog, cx| dialog.focus(window, cx));
        }
        let on_ok = on_ok.clone();
        let on_cancel = on_cancel.clone();
        shell
            .title("Password required")
            .w(px(DIALOG_WIDTH))
            .overlay_closable(false)
            .on_ok(move |_, _, cx| {
                on_ok.update(cx, |dialog, cx| dialog.submit(cx));
                true
            })
            .on_cancel(move |_, _, cx| {
                on_cancel.update(cx, |dialog, cx| dialog.cancel(cx));
                true
            })
            .child(content.clone())
    });
    Some(dialog)
}

impl PasswordDialog {
    fn new(
        state: Entity<UiState>,
        sink: CommandSink,
        connection_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        Self {
            state,
            sink,
            connection_name,
            password,
            save: false,
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.password
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn set_save(&mut self, save: bool, cx: &mut Context<Self>) {
        self.save = save;
        cx.notify();
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        // @NOTE: the input keeps its own copy of the text; this only limits ours.
        let password = SecretString::from(self.password.read(cx).value().to_string());
        let save = self.save;
        let commands = self.state.update(cx, |state, cx| {
            cx.notify();
            state.submit_password(password, save)
        });
        client::dispatch(&self.sink, &self.state, commands, cx);
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.cancel_password();
            cx.notify();
        });
    }

    #[cfg(test)]
    pub fn input(&self) -> &Entity<InputState> {
        &self.password
    }
}

impl Render for PasswordDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let save = Checkbox::new("remember-password")
            .label("Save password")
            .checked(self.save)
            .on_click(cx.listener(|dialog, checked: &bool, _, cx| {
                dialog.set_save(*checked, cx);
            }));
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!(
                        "Enter the password for \"{}\". It is used for this session unless you save it.",
                        self.connection_name
                    ))),
            )
            .child(Input::new(&self.password).mask_toggle())
            .child(save)
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-password")
                            .label("Cancel")
                            .outline()
                            .small()
                            .on_click(cx.listener(|dialog, _, window, cx| {
                                dialog.cancel(cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("connect-password")
                            .label("Connect")
                            .primary()
                            .small()
                            .on_click(cx.listener(|dialog, _, window, cx| {
                                dialog.submit(cx);
                                window.close_dialog(cx);
                            })),
                    ),
            )
    }
}
