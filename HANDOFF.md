# Handoff

## Done
- Workspace skeleton: `dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app` (binary `dbased`); fmt, clippy, build pass.
- Week-one spike items 6-8 (tokenizer, keyring, driver gotchas) done and folded into `docs/PROJECT.md` (revision log, 2026-11 spike entry). Key changes: keep a session after a clean cancel (3.2), sessions outside the pool (2.2), one statement per `raw_sql` call over the text protocol (2.3, 2.5, 9.2), in-house scanner as the only lexer (2.7), keyring wiring decided for Linux (5.1).

## In progress
- Nothing.

## Next step
- Spike items 1-5 (grid, editor, IME/Wayland, startup, Dock) in a graphical session.
- First real driver (Postgres): apply the 2.1 trait (Session / Canceller, `UserSql`, `health_check`), put the `AssertSqlSafe` invariant in the `UserSql` conversion function (1.2, rule 1), and implement the 3.2 cancel policy with integration tests for: clean cancel, late cancel, aborted transaction, drain timeout.

## Re-verify before relying on it
- Dropped-future and cancel behavior on the extended protocol / `prepare` / sqlx `begin()` (spike covered simple-protocol queries only).
- MySQL zero-date in the binary protocol: `is_null()` true, or empty bytes?
- Keyring calls from `spawn_blocking` inside the Core tokio runtime with `rt-tokio-crypto-rust`, and building entries from a store instance instead of the default store.
- TLS `verify-ca` / `verify-full` against native roots and a real CA bundle.
- Tokenizer matrix additions in 2.7 (MySQL `--` spacing, backslash escapes, `/*! */`, Postgres `BEGIN ATOMIC`, `a[1:n]`).

## Open questions
- Default keymap preset (recommendation: DBeaver-like).
- Swap `directories` for `etcetera` in the workspace manifest; `sqlparser`, `toml`, `serde`-in-core are declared but unused.
- `unused_dependencies` lint is allowed workspace-wide until ui/app use their declared deps.
- Default TLS mode for new connections is not stated in 5.3.
