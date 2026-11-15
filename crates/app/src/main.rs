use std::process::ExitCode;

#[cfg(not(feature = "fake-core"))]
use std::sync::Arc;
use std::time::Duration;

use dbased_commands::{COMMAND_CAPACITY, CoreLink, EVENT_CAPACITY, bridge};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// The Core's drain timeout plus a margin (docs/PROJECT.md 11.3).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(12);

fn main() -> ExitCode {
    // @NOTE: GPUI owns the main thread, so database work runs on the runtime's own workers.
    let runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);
    let core_task = start_core(&runtime, core_link);

    dbased_ui::run(ui_link);

    // @NOTE: the UI link is gone, so the Core sees the closed channel and shuts down on its own.
    if runtime
        .block_on(tokio::time::timeout(SHUTDOWN_BUDGET, core_task))
        .is_err()
    {
        eprintln!("the Core did not shut down within {SHUTDOWN_BUDGET:?}; exiting anyway");
    }
    ExitCode::SUCCESS
}

#[cfg(feature = "fake-core")]
fn start_core(runtime: &Runtime, core_link: CoreLink) -> JoinHandle<()> {
    runtime.spawn(dbased_commands::fake::FakeCore::new().serve(core_link))
}

#[cfg(not(feature = "fake-core"))]
fn start_core(runtime: &Runtime, core_link: CoreLink) -> JoinHandle<()> {
    use dbased_commands::{ErrorInfo, ErrorKind, Event};
    use dbased_core::{
        ConnectionStore, Core, CoreDependencies, DefaultConnector, InMemorySecretStore,
        KeychainSecretStore, SecretStore,
    };

    let _runtime_guard = runtime.enter();
    let mut startup_errors = Vec::new();

    let secret_store: Arc<dyn SecretStore> = match KeychainSecretStore::new() {
        Ok(store) => Arc::new(store),
        Err(error) => {
            // @NOTE: without a keychain passwords live only for this session (docs/PROJECT.md 5.1).
            startup_errors.push(ErrorInfo {
                kind: ErrorKind::SecretStore,
                message: format!(
                    "the system keychain is unavailable ({error}); passwords will be asked once per session"
                ),
                database: None,
            });
            Arc::new(InMemorySecretStore::default())
        }
    };

    let connection_store = match ConnectionStore::default_path() {
        Ok(path) => ConnectionStore::new(path),
        Err(error) => {
            // @NOTE: a temp-dir file keeps the app usable; the notice says nothing will persist reliably.
            startup_errors.push(ErrorInfo {
                kind: ErrorKind::Storage,
                message: format!(
                    "no config directory ({error}); connections are saved to a temporary file"
                ),
                database: None,
            });
            ConnectionStore::new(std::env::temp_dir().join("dbased").join("config.toml"))
        }
    };

    for error in startup_errors {
        // @NOTE: try_send because the UI is not pumping yet; the channel is empty so it fits.
        let _ = core_link.events.try_send(Event::Error { error });
    }

    let core = Core::new(CoreDependencies {
        connection_store,
        secret_store,
        connector: Arc::new(DefaultConnector),
    });
    runtime.spawn(core.run(core_link))
}
