# Handoff

## Done
- Workspace skeleton: `dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app` (binary `dbased`).
- `commands`: `Command`, `Event`, `JobId`, `Value`, placeholder `ConnectionConfig`.
- `core`: `CoreError` and `Driver` trait skeleton (no implementations yet).
- `ui`: empty gpui-component window via `run()`; `app` starts tokio on a background thread and calls it.
- fmt, clippy (-D warnings) and build pass. Dependency rule verified with `cargo tree`.

## In progress
- Nothing.

## Next step
- Implement the SQLite driver (simplest) behind `Driver`, then wire Command/Event channels between ui and core.

## Open questions
- keyring 4.x is declared in `[workspace.dependencies]` but unused. It split into `keyring-core` plus credential-store crates; decide the store setup before first use.
- `sqlparser`, `directories`, `toml`, `serde`-in-core are declared centrally but not yet used by any crate.
- `unused_dependencies` cargo lint is allowed workspace-wide until ui/app use their declared deps.
- The window has not been launched in a display session yet (build only).
