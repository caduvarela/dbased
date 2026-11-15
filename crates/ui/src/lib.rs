//! The only crate allowed to use GPUI. Talks to the core exclusively via `dbased-commands`.

mod actions;
mod client;
mod editor_panel;
mod explorer_panel;
mod grid;
mod results_panel;
mod state;
mod workspace;

#[cfg(test)]
mod tests;

use dbased_commands::UiLink;
use gpui::{AppContext as _, TitlebarOptions, WindowOptions};
use gpui_component::Root;
use gpui_kit_assets::Assets;

use workspace::Workspace;

/// Registers what the UI needs at app level: components, key bindings.
fn init(cx: &mut gpui::App) {
    gpui_component::init(cx);
    actions::bind_keys(cx);
}

/// Opens the main window on `link` and runs until it closes.
pub fn run(link: UiLink) {
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx| {
            init(cx);
            cx.on_window_closed(|cx, _window| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("DBased".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| Workspace::new(link, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("failed to open window: {error}");
                cx.quit();
            }
        });
}
