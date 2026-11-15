# Handoff

## Done
- Workspace skeleton: `dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app` (binary `dbased`).
- `commands`: `Command`, `Event`, `JobId`, `Value`, placeholder `ConnectionConfig`.
- `core`: `CoreError` and `Driver` trait skeleton (no implementations yet).
- `ui`: empty gpui-component window via `run()`; `app` starts tokio on a background thread and calls it.
- fmt, clippy (-D warnings) and build pass. Dependency rule verified with `cargo tree`.
- `docs/PROJECT.md`: Constitution + Architecture Decisions, re-verified against the ecosystem on 2026-11-15 (see its revision log).

## In progress
- Nothing.

## Next step
- Run the week-one spike (docs/PROJECT.md, Part 10) before building the real UI: 5M-row windowed grid, 100k-line editor with multi-cursor, IME/Wayland, startup time, Dock round-trip, sqlparser tokenizer behavior, keyring stores, driver gotchas (cancel, raw_sql, Value matrix).
- Then apply the Session/Canceller split to the `Driver` trait (2.1) with the first real driver.
- With the first real driver, replace the provisional `@NOTE` about `AssertSqlSafe` above the `Driver` trait with a `UserSql` newtype and one conversion function per driver carrying a short, version-independent comment (docs/PROJECT.md, 1.2, rule 1).

## Open questions
- Default keymap preset (recommendation: DBeaver-like).
- Keyring store wiring and the Linux no-Secret-Service fallback (resolved by the spike).
- Swap `directories` for `etcetera` in the workspace manifest; `sqlparser`, `toml`, `serde`-in-core are declared but unused.
- `unused_dependencies` lint is allowed workspace-wide until ui/app use their declared deps.
- The window has not been launched in a display session yet (build only).
