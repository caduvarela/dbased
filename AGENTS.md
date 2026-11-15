# DBased

Native desktop SQL client in Rust (GPUI + gpui-component, sqlx, PostgreSQL + MySQL for the MVP).
Goal: the SQL client developers enjoy using every day. Fast, clean, predictable.

## Start every session by reading

1. `docs/PROJECT.md`: Constitution (Part A) and Architecture Decisions (Part B). Part B wins on technical specifics, Part A wins on philosophy.
2. `HANDOFF.md`: current working state and the next concrete step.

Durable decisions go in `docs/PROJECT.md`, never only in `HANDOFF.md`. Update `HANDOFF.md` at every milestone and before ending a session. It must never reach `master`.

## Language

- Talk to the user in Brazilian Portuguese.
- Everything written to the repo is English: code, comments, docs, commits, PR text.

## Workspace

```
crates/commands  dbased-commands  Command/Event enums and shared DTOs (the contract)
crates/sql       dbased-sql       pure SQL scanner, statement splitter, parameter finder (no deps)
crates/core      dbased-core      business logic, sqlx drivers, jobs, cache
crates/ui        dbased-ui        GPUI + gpui-component, depends only on `commands`
crates/app       dbased-app       binary `dbased`, wires core and ui
```

Hard rules (enforced by the compiler and checked with `cargo tree`):

- `core` never imports `gpui` or `gpui-component`. `ui` never depends on `core`. `sql` depends on nothing.
- UI and Core talk only through `Command`/`Event` values from `commands`.
- tokio and sqlx stay inside `core` (and `app` for starting the runtime).
- Pin every `gpui-*` crate with an exact `=x.y.z`. Upgrade deliberately, on its own branch.
- Declare a dependency in a crate only if that crate uses it.

## Commands

Linux build prerequisites (Debian/Ubuntu): `build-essential pkg-config cmake libfontconfig1-dev libfreetype-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libxcb1-dev libx11-xcb-dev libxcb-xkb-dev libasound2-dev libssl-dev libvulkan1 mesa-vulkan-drivers`. Run the app with `cargo run -p dbased-app` (binary `dbased`).

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace                     # plain runner, no extra tools
cargo nextest run --workspace              # preferred runner (profiles: default, ci)
cargo test --workspace --doc               # doctests (nextest does not run them)
cargo tree -p dbased-core | grep gpui      # must print nothing
cargo tree -p dbased-ui | grep dbased-core # must print nothing
```

Run fmt, clippy and tests before every commit. Rust toolchain is pinned in `rust-toolchain.toml`.

Integration tests (real Postgres/MySQL via testcontainers) live behind the `integration` cargo feature of `dbased-core` (`cargo test -p dbased-core --features integration`, needs Docker; images come from `.env`).

Manual dev databases with sample data (never used by tests; see `dev/README.md`):

```
docker compose up -d     # Postgres on 127.0.0.1:54320, MySQL on 127.0.0.1:33060
docker compose down -v   # stop and wipe data
```

## Code rules

- No `unwrap()` / `expect()` / `unsafe` outside tests (clippy-enforced).
- Readable full names (`connection`, not `conn`). Early returns, small functions, intermediate variables over clever chains.
- Comments explain WHY only, as `// @NOTE:`, `// @TODO:` or `// @FIXME:`. Keep them short and independent of library versions.
- Prefer traits and composition. No singletons or global mutable state. No unnecessary abstractions.
- User-typed SQL is the only SQL wrapped in `AssertSqlSafe`, through the `UserSql` type (docs/PROJECT.md 1.2). Never use `sqlx::Any`. Never rely on dropping a sqlx future to cancel a query.
- Never log or `Debug`-print secrets. Hold them in `secrecy::SecretString`.

## Git

- Conventional Commits (`feat(query): ...`, `fix(metadata): ...`, `docs(architecture): ...`). One logical change per commit, commit often.
- One feature per pull request. Do not push or open PRs unless asked.

## Working style

- Verify library facts against live sources (crates.io, release notes) before relying on memory: this ecosystem moves fast and GPUI is pre-1.0.
- Significant architectural changes must state why they help, which pillar they serve, and the trade-offs.
- Build only what the current milestone needs. If a requirement is ambiguous, pick the simplest option that respects the Constitution.
