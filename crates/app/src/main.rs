use std::process::ExitCode;

use dbased_commands::{COMMAND_CAPACITY, CoreLink, EVENT_CAPACITY, bridge};

fn main() -> ExitCode {
    // @NOTE: GPUI owns the main thread, so database work runs on the runtime's own workers.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);
    start_core(&runtime, core_link);

    dbased_ui::run(ui_link);
    ExitCode::SUCCESS
}

#[cfg(feature = "fake-core")]
fn start_core(runtime: &tokio::runtime::Runtime, core_link: CoreLink) {
    runtime.spawn(dbased_commands::fake::FakeCore::new().serve(core_link));
}

// @TODO: start the real Core here once it can be built (docs/PROJECT.md 11.3).
#[cfg(not(feature = "fake-core"))]
fn start_core(_runtime: &tokio::runtime::Runtime, core_link: CoreLink) {
    // @NOTE: dropping the link closes the channels, so the UI shows a stopped Core.
    drop(core_link);
}
