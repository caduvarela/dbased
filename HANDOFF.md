# Handoff

## Done
- Workspace skeleton: `dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app` (binary `dbased`); fmt, clippy, build pass.
- Dev compose (`compose.yaml`, `.env`, `dev/seed/`, `dev/README.md`): Postgres 17 and MySQL 8.4 with seeded data incl. a 1M-row table.
- Week-one spike items 6-8 (tokenizer, keyring, driver gotchas) folded into `docs/PROJECT.md`.
- Week-one spike items 1, 2, 4, 5 (grid, editor, startup, Dock) DONE, item 3 (IME) PARTIAL, folded into `docs/PROJECT.md` (revision log, GUI spike entry; 0.1, 0.4, 5.2, 6.1-6.3, 9.2, Part 10). Spike code lives outside the repo in `../dbased-spike`. Gate passed for starting the real UI.

## In progress
- Nothing.

## Next step
- First real driver (Postgres): the 2.1 trait (Session / Canceller, `UserSql`, `health_check`), the `AssertSqlSafe` invariant in the `UserSql` conversion (1.2, rule 1), the 3.2 cancel policy with integration tests (clean cancel, late cancel, aborted transaction, drain timeout).
- In parallel or next: real UI shell in `crates/ui` (Dock with explorer / editor / bottom result tabs, grid against a fake Core source) following the 6.3 grid rules. Add `tree-sitter-sql` and `gpui-base =0.9.1` to `ui` only when first used.

## Pending manual checks (Part 10 gate status)
- Item 3 IME checklist on X11 and Wayland (`../dbased-spike` README, section 3).
- Look at the grid by eye near the end of 5M rows, with wheel and scrollbar drag.
- Items 1-5 on Wayland; grid on lower-end hardware and under GPU contention.
- Startup with dropped caches (cold).

## Re-verify before relying on it
- Grid 60 fps on a quiet machine: the persisted spike logs (`../dbased-spike/results.txt`, `logs/grid*.log`) only contain the 33 ms contention runs; record a clean run.
- Editor RSS growth (127 -> ~295-385 MB on 100K lines): one-time or per-edit?
- Dropped-future and cancel behavior on the extended protocol / `prepare` / sqlx `begin()`.
- MySQL zero-date in the binary protocol: `is_null()` true, or empty bytes?
- Keyring calls from `spawn_blocking` inside the Core runtime; TLS `verify-ca` / `verify-full` against real roots.
- Tokenizer matrix additions in 2.7.

## Open questions
- Default keymap preset (recommendation: DBeaver-like).
- Swap `directories` for `etcetera` in the workspace manifest; `sqlparser`, `toml`, `serde`-in-core are declared but unused.
- `unused_dependencies` lint is allowed workspace-wide until ui/app use their declared deps.
