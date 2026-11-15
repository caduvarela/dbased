# Handoff

Updated 2026-11-15. Read `docs/PROJECT.md` first (Part A constitution, Part B decisions; Parts 10 and 11 are the freshest). This file must never reach `master`.

## Where the code stands

Stacked PRs already open on GitHub (HANDOFF-free copies of history): #1 scaffold + docs, #2 Postgres driver, #3 `dbased-sql`. Everything below is newer, local-history work on feature branches (not in those PRs yet).

Branch topology (each line is "based on"):
- `feat/core-contract` (Part 11 spec, contract in `commands`, dev profile tweak)
  - `feat/core-service-skeleton` (dispatcher, ConnectionStore TOML, SecretStore trait, error mapping, shutdown)
    - `feat/core-execution` (tab actors, statement execution, result store, `dbased-sql::statement_at`)
      - `fix/early-columns` (Postgres sends Columns before the statement runs, outside transactions)
      - `feat/csv-export` (done, not merged, see below)
    - `feat/keychain-store` (keyring-backed SecretStore)
  - `feat/fake-core` (FakeCore in `commands`, feature `fake`)
    - `feat/ui-shell` (dock, explorer, editor tabs, results grid, 69 tests)
      - `feat/ui-connections` (connection dialog, password prompt, delete confirm)
- `feat/app-wiring` = integration branch: merges keychain, ui-shell/fake-core, ui-connections, early-columns on top of core-execution; real Core wired in `crates/app` (bounded 12 s shutdown); contract tests run the same scenarios against FakeCore and the real Core. **This is the best branch to build and run.**
  - `fix/execution-review` (merged into this branch)
- `feat/mysql-driver` (done, not merged; based on an older commit `ad515c0`)

## Status of the side branches (2026-11-15, end of session)
- `fix/execution-review`: DONE and MERGED into `feat/app-wiring` (all 8 review findings fixed, 13 new tests, integration suite green twice). Choices: cancel before start ends as `ExecutionStarted` then `ExecutionFinished(Cancelled{0})`; bad `Selection` range maps to `ErrorKind::Configuration`; Disconnect while connecting still returns `Internal`.
- `feat/csv-export`: DONE (Core only), pushed, NOT yet merged. Merging it into `feat/app-wiring` conflicts in `service/dispatcher.rs`, `service/mod.rs` and `service/tests_session.rs` (the review fixes reworked the dispatcher). Resolve by hand: keep the review-fix structure (task-id role map, plan before claiming the slot) and re-add the export registry, `ExportCsv`/`CancelExport` handling and export shutdown/disconnect cancellation.
- `feat/mysql-driver`: DONE (34 integration tests), pushed, NOT yet merged (also based on the old commit `ad515c0`; expect conflicts in `drivers/mod.rs`, `tests/support/mod.rs`, workspace manifests). Not wired into `DefaultConnector`.

## Next steps (suggested order)
1. Merge `feat/csv-export`, then `feat/mysql-driver`, into `feat/app-wiring` (conflicts described above), then re-run fmt, clippy (also with `-p dbased-core --features integration`) and both test suites.
2. Record in `docs/PROJECT.md` (11.2/3.2.4) the deviations found by the review: dispatcher claims the running slot (not the actor); `Disconnected` is emitted before sessions finish closing; planning/EmptyScript moves to the dispatcher; `ResultBuffer` holds rows only; `StatementSpan.index` counts within the plan; MySQL `Selection` loses a `DELIMITER` above the selection; cancel-before-start terminal event choice.
3. Wire the MySQL driver into `DefaultConnector` (currently `NotImplemented`), `list_routines` for both drivers.
4. UI: CSV export action, simple row editing (2.6), command palette, global search, editor error underline from `DatabaseError.position`, layout persistence.
5. Manual end-to-end run (`docker compose up -d`, then `cargo run -p dbased-app` on `feat/app-wiring`): connect, run a script, cancel, quit cleanly. Not yet done by a human.
6. Open the next round of stacked PRs (contract, fake core, skeleton, execution, keychain, UI shell, connections, wiring, MySQL, CSV) once reviewed. Remember: PR branches must exclude `HANDOFF.md` (filter it out of a copy of the history; remote `chore/workspace-scaffold` still contains it and should not be merged).

## Known issues / follow-ups
- Flaky timing test `a_row_cap_outside_a_transaction_still_cancels_after_commit_and_rollback` (10 s bound under load): loosen to 30 s.
- `Execute` for a PasswordRequired connection is only tested at logic level in the UI (FakeCore never returns it).
- macOS, Windows and KWallet keyring paths were never compiled or tested.
- Pending manual checks (Part 10): CJK composition with a real input method, Wayland, lower-end hardware, cold startup, clean 60 fps grid run recorded, editor RSS growth.

## Environment notes
- Rust toolchain pinned in `rust-toolchain.toml`; `source ~/.cargo/env` if cargo is not in PATH. Linux build needs the apt `-dev` packages listed in `AGENTS.md`.
- `target/` can reach 15+ GB. Dev profile now uses `debug = "line-tables-only"` and no debug info for dependencies. `cargo-sweep` is installed locally (`cargo sweep --stamp`, rebuild, `cargo sweep --file`). Parallel worktrees should share one `CARGO_TARGET_DIR`; if cargo reports a stale "unresolved import", touch the crate's `lib.rs`.
- Dev databases: `docker compose up -d` (Postgres 17 on 54320, MySQL 8.4 on 33060, credentials `dbased`/`dbased`). Integration tests use testcontainers behind the `integration` feature and never touch these.
- Agent workflow used so far: Sonnet subagents implement in git worktrees under `../dbased-wt/<name>`; Opus agents design and review. Commits are single-line Conventional Commits, no body, no trailers. Never push or open PRs unprompted.

## Open questions (user)
- Names and contents of the 3-4 formatting presets (9.6).
- Run shortcuts chosen by the UI agent (Ctrl+Enter runs selection/current, Ctrl+Shift+Enter runs all): confirm.
- `sqlparser`, `toml` and similar unused workspace deps remain declared; the `unused_dependencies` lint is allowed workspace-wide.
