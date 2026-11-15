# Handoff

## Done
- Workspace skeleton: `dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app` (binary `dbased`); fmt, clippy, build pass. `etcetera` and the `keyring-core` stores are in the manifest.
- Dev compose (`compose.yaml`, `.env`, `dev/seed/`, `dev/README.md`): Postgres 17 and MySQL 8.4 with seeded data incl. a 1M-row table.
- Week-one spike (items 1-8) and the 2026-11 human tests on X11/MATE folded into `docs/PROJECT.md` (Part 10, 6.1, 7.1). Spike code lives outside the repo in `../dbased-spike`. Gate passed for starting the real UI.
- Recorded as Later (not next): VS Code-like default keymap, VS Code Windows mapping confirmed (9.4), smart multi-cursor (9.4.1), step-by-step debug mode (9.5), SQL formatting with portable styles (9.6), explain visualization (9.7), own emoji/symbol picker (6.1).

## In progress
- Nothing.

## Next step
- Core service (Part 11): spec written and the six product calls decided. Implement per 11.5, starting with PR 1 (the contract in `commands`).
- `dbased-sql` crate done (cbd22cd..1649306): scanner, statement splitter, parameter finder; not wired into core yet. Wire the splitter into the Core execution path (9.2) so multi-statement strings never reach `raw_sql`, then `rewrite_parameters`.
- Postgres driver slice is done and reviewed (commits up to a709f08). Decisions taken (3.2.4): drain-not-cancel inside user transactions; cancel handle tied to a live session. Both implemented (6d66a6f, 0c672dd); 33 integration tests.
- First real driver (Postgres): the 2.1 trait (Session / Canceller, `UserSql`, `health_check`), the `AssertSqlSafe` invariant in the `UserSql` conversion (1.2, rule 1), the 3.2 cancel policy with integration tests (clean cancel, late cancel, aborted transaction, drain timeout).
- In parallel or next: real UI shell in `crates/ui` (Dock with explorer / editor / bottom result tabs, grid against a fake Core source) following the 6.3 grid rules; the editor takes focus on open and tab activation (6.1). Add `tree-sitter-sql` and `gpui-base =0.9.1` to `ui` only when first used.
- Not next: the keymap commands, smart multi-cursor and the Later features above.

## Pending manual checks (Part 10 gate status)
- CJK composition with a real input method (XIM on X11: `XMODIFIERS` plus ibus/fcitx5).
- Items 1-5 on Wayland; grid on lower-end hardware and under GPU contention.
- Startup with dropped caches (cold).

## Re-verify before relying on it
- Grid 60 fps on a quiet machine: the persisted spike logs only contain the 33 ms contention runs; record a clean run.
- Editor RSS growth (127 -> ~295-385 MB on 100K lines): one-time or per-edit?
- Dropped-future and cancel behavior on the extended protocol / `prepare` / sqlx `begin()`.
- MySQL zero-date in the binary protocol: `is_null()` true, or empty bytes?
- Keyring calls from `spawn_blocking` inside the Core runtime; TLS `verify-ca` / `verify-full` against real roots.
- Tokenizer matrix additions in 2.7.
- An input method that uses `Ctrl+.` sees the key before the app (XIM, Wayland); `Ctrl+.` stays upstream code actions.
- Upstream Ctrl+Alt+click (no drag) adds a cursor on Linux; MySQL 8.4 JSON output for `EXPLAIN ANALYZE`.

## Open questions (user)
- (Resolved 2026-11: `Ctrl+.` stays code actions; `dbased-sql` crate approved.)
- Names and contents of the 3-4 formatting presets (9.6).
- `sqlparser`, `toml`, `serde`-in-core are declared but unused; `unused_dependencies` lint is allowed workspace-wide until ui/app use their declared deps.
