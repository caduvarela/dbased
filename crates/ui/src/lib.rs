//! The only crate allowed to use GPUI. Talks to the core exclusively via `dbased-commands`.

use gpui::{AppContext, TitlebarOptions, WindowOptions};
use gpui_component::Root;

pub fn run() {
    gpui_platform::application().run(|app| {
        gpui_component::init(app);
        app.spawn(async move |app| {
            let open_result = app.update(|app| {
                let options = WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("DBased".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                app.open_window(options, |window, app| {
                    let view = app.new(|_| EmptyView);
                    app.new(|app| Root::new(view, window, app))
                })
            });
            if let Err(error) = open_result {
                eprintln!("failed to open window: {error}");
            }
        })
        .detach();
    });
}

struct EmptyView;

impl gpui::Render for EmptyView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _app: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}
