use std::cell::Cell;

use dbased_commands::{DatabaseKind, TlsMode};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use secrecy::SecretString;

use crate::{
    client::{self, CommandSink},
    state::{
        ConnectionForm, FormErrors, SaveStatus, TestStatus, UiState, kind_label, tls_label,
        tls_warning,
    },
};

const DIALOG_WIDTH: f32 = 480.0;
const LABEL_WIDTH: f32 = 84.0;

/// The create/edit connection dialog (docs/PROJECT.md 5.1, 5.3, 11.1). The text lives
/// in input entities; everything else is the pure `ConnectionForm`.
pub struct ConnectionDialog {
    state: Entity<UiState>,
    sink: CommandSink,
    form: ConnectionForm,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    database: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    /// Shown after the first failed submit, so a fresh form is not covered in errors.
    errors: Option<FormErrors>,
    _subscriptions: Vec<Subscription>,
}

/// Opens the dialog; `form` is `ConnectionForm::new()` or an existing connection's.
pub fn open(
    state: Entity<UiState>,
    sink: CommandSink,
    form: ConnectionForm,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConnectionDialog> {
    state.update(cx, |state, cx| {
        state.flow.reset_dialog();
        cx.notify();
    });
    let title = if form.editing().is_some() {
        "Edit connection"
    } else {
        "New connection"
    };
    let dialog = cx.new(|cx| ConnectionDialog::new(state, sink, form, window, cx));
    let content = dialog.clone();
    let on_ok = dialog.clone();
    let initial_focus = Cell::new(true);
    window.open_dialog(cx, move |shell, window, cx| {
        if initial_focus.replace(false) {
            content.update(cx, |dialog, cx| dialog.focus_first(window, cx));
        }
        let on_ok = on_ok.clone();
        shell
            .title(title)
            .w(px(DIALOG_WIDTH))
            .overlay_closable(false)
            .on_ok(move |_, window, cx| {
                // @NOTE: the dialog closes itself once the Core confirms the save.
                on_ok.update(cx, |dialog, cx| dialog.submit(window, cx));
                false
            })
            .child(content.clone())
    });
    dialog
}

impl ConnectionDialog {
    fn new(
        state: Entity<UiState>,
        sink: CommandSink,
        form: ConnectionForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_input = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let name = text_input(&form.name, window, cx);
        let host = text_input(&form.host, window, cx);
        let port = text_input(&form.port, window, cx);
        let database = text_input(&form.database, window, cx);
        let user = text_input(&form.user, window, cx);
        let placeholder = if form.password_was_saved() {
            "Leave empty to keep the saved password"
        } else {
            ""
        };
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(placeholder)
        });
        let mut subscriptions = vec![cx.observe_in(&state, window, |dialog, _, window, cx| {
            dialog.state_changed(window, cx);
        })];
        for input in [&name, &host, &port, &database, &user, &password] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |dialog, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        dialog.edited(cx);
                    }
                },
            ));
        }
        Self {
            state,
            sink,
            form,
            name,
            host,
            port,
            database,
            user,
            password,
            errors: None,
            _subscriptions: subscriptions,
        }
    }

    fn focus_first(&self, window: &mut Window, cx: &mut App) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.flow.clear_test_result();
            cx.notify();
        });
    }

    fn state_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = matches!(self.state.read(cx).flow.save, SaveStatus::Saved(_));
        if saved {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    /// The form as it stands now: the pure part plus what the inputs hold.
    pub fn collect(&self, cx: &App) -> ConnectionForm {
        let mut form = self.form.clone();
        form.name = self.name.read(cx).value().to_string();
        form.host = self.host.read(cx).value().to_string();
        form.port = self.port.read(cx).value().to_string();
        form.database = self.database.read(cx).value().to_string();
        form.user = self.user.read(cx).value().to_string();
        // @NOTE: the input keeps its own copy of the text; this only limits ours.
        form.password = SecretString::from(self.password.read(cx).value().to_string());
        form
    }

    pub fn set_kind(&mut self, kind: DatabaseKind, window: &mut Window, cx: &mut Context<Self>) {
        let mut form = self.collect(cx);
        form.set_kind(kind);
        let port = form.port.clone();
        self.form.kind = kind;
        self.port
            .update(cx, |input, cx| input.set_value(port, window, cx));
        cx.notify();
    }

    pub fn set_tls(&mut self, tls: TlsMode, cx: &mut Context<Self>) {
        self.form.tls = tls;
        cx.notify();
    }

    pub fn set_save_password(&mut self, save: bool, cx: &mut Context<Self>) {
        self.form.save_password = save;
        cx.notify();
    }

    pub fn test(&mut self, cx: &mut Context<Self>) {
        let form = self.collect(cx);
        let result = self.state.update(cx, |state, cx| {
            cx.notify();
            state.test_connection(&form)
        });
        self.send(result, cx);
    }

    pub fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let form = self.collect(cx);
        let result = self.state.update(cx, |state, cx| {
            cx.notify();
            state.save_connection(&form)
        });
        self.send(result, cx);
    }

    fn send(
        &mut self,
        result: Result<Vec<dbased_commands::Command>, FormErrors>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(commands) => {
                self.errors = None;
                client::dispatch(&self.sink, &self.state, commands, cx);
            }
            Err(errors) => self.errors = Some(errors),
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn errors(&self) -> Option<&FormErrors> {
        self.errors.as_ref()
    }

    #[cfg(test)]
    pub fn input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Name => &self.name,
            Field::Host => &self.host,
            Field::Port => &self.port,
            Field::Database => &self.database,
            Field::User => &self.user,
            Field::Password => &self.password,
        }
    }

    fn field(
        &self,
        label: &'static str,
        input: AnyElement,
        error: Option<&'static str>,
        cx: &App,
    ) -> AnyElement {
        let danger = cx.theme().danger;
        v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(LABEL_WIDTH))
                            .flex_shrink_0()
                            .text_sm()
                            .child(label),
                    )
                    .child(div().flex_1().child(input)),
            )
            .children(error.map(|message| {
                div()
                    .pl(px(LABEL_WIDTH + 8.0))
                    .text_xs()
                    .text_color(danger)
                    .child(message)
            }))
            .into_any_element()
    }

    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        selected: T,
        on_pick: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut row = h_flex().gap_1();
        for (index, (value, label)) in options.iter().copied().enumerate() {
            let on_pick = on_pick.clone();
            let button = Button::new(SharedString::from(format!("{id}-{index}")))
                .label(label)
                .small()
                .on_click(cx.listener(move |dialog, _, window, cx| {
                    on_pick(dialog, value, window, cx);
                }));
            let button = if value == selected {
                button.primary()
            } else {
                button.outline()
            };
            row = row.child(button);
        }
        row.into_any_element()
    }

    fn render_test_result(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (text, color) = match &self.state.read(cx).flow.test {
            TestStatus::Idle => return div().into_any_element(),
            TestStatus::Testing => ("Testing...".to_string(), theme.muted_foreground),
            TestStatus::Success => ("Connection successful".to_string(), theme.success),
            TestStatus::Failed(message) => (message.clone(), theme.danger),
        };
        div()
            .text_sm()
            .text_color(color)
            .child(SharedString::from(text))
            .into_any_element()
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub enum Field {
    Name,
    Host,
    Port,
    Database,
    User,
    Password,
}

impl Render for ConnectionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let errors = self.errors.clone().unwrap_or_default();
        let (testing, saving) = {
            let flow = &self.state.read(cx).flow;
            (
                flow.test == TestStatus::Testing,
                flow.save == SaveStatus::Saving,
            )
        };
        let save_error = match &self.state.read(cx).flow.save {
            SaveStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let kinds = [
            (DatabaseKind::Postgres, kind_label(DatabaseKind::Postgres)),
            (DatabaseKind::MySql, kind_label(DatabaseKind::MySql)),
        ];
        let tls_modes = [TlsMode::Require, TlsMode::Prefer, TlsMode::Disable]
            .map(|mode| (mode, tls_label(mode)));
        let driver = self.choice(
            "driver",
            &kinds,
            self.form.kind,
            |dialog, kind, window, cx| dialog.set_kind(kind, window, cx),
            cx,
        );
        let tls = self.choice(
            "tls",
            &tls_modes,
            self.form.tls,
            |dialog, mode, _, cx| dialog.set_tls(mode, cx),
            cx,
        );
        let warning = tls_warning(self.form.tls);
        let warning_color = cx.theme().warning;
        let danger = cx.theme().danger;
        let save_password = Checkbox::new("save-password")
            .label("Save password")
            .checked(self.form.save_password)
            .on_click(cx.listener(|dialog, checked: &bool, _, cx| {
                dialog.set_save_password(*checked, cx);
            }));

        v_flex()
            .gap_2()
            .child(self.field(
                "Name",
                Input::new(&self.name).into_any_element(),
                errors.name,
                cx,
            ))
            .child(self.field("Driver", driver, None, cx))
            .child(self.field(
                "Host",
                Input::new(&self.host).into_any_element(),
                errors.host,
                cx,
            ))
            .child(self.field(
                "Port",
                Input::new(&self.port).into_any_element(),
                errors.port,
                cx,
            ))
            .child(self.field(
                "Database",
                Input::new(&self.database).into_any_element(),
                None,
                cx,
            ))
            .child(self.field(
                "User",
                Input::new(&self.user).into_any_element(),
                errors.user,
                cx,
            ))
            .child(self.field(
                "Password",
                Input::new(&self.password).mask_toggle().into_any_element(),
                None,
                cx,
            ))
            .child(h_flex().pl(px(LABEL_WIDTH + 8.0)).child(save_password))
            .child(self.field("TLS", tls, None, cx))
            .children(warning.map(|text| {
                div()
                    .pl(px(LABEL_WIDTH + 8.0))
                    .text_xs()
                    .text_color(warning_color)
                    .child(text)
            }))
            .child(self.render_test_result(cx))
            .children(save_error.map(|message| {
                div()
                    .text_sm()
                    .text_color(danger)
                    .child(SharedString::from(message))
            }))
            .child(
                h_flex()
                    .justify_between()
                    .pt_2()
                    .child(
                        Button::new("test-connection")
                            .label("Test connection")
                            .outline()
                            .disabled(testing)
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.test(cx))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel-connection")
                                    .label("Cancel")
                                    .outline()
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("save-connection")
                                    .label("Save")
                                    .primary()
                                    .disabled(saving)
                                    .on_click(cx.listener(|dialog, _, window, cx| {
                                        dialog.submit(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
