use std::process::ExitCode;

fn main() -> ExitCode {
    // @NOTE: GPUI owns the main thread, so the async runtime lives on a background thread.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    std::thread::spawn(move || {
        runtime.block_on(std::future::pending::<()>());
    });

    dbased_ui::run();
    ExitCode::SUCCESS
}
