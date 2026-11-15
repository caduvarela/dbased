# DBased — Project Documentation

Single source of truth for the DBased project: the Constitution (why, and
what kind of project this is) and the Architecture Decisions (what was
decided, and how it works).

---

## How to read this document

- **Part A, Constitution.** Durable principles and philosophy. Changes
  rarely, and only deliberately.
- **Part B, Architecture Decisions.** Concrete technical decisions, each
  written as **Decision / Why / Trade-off**. Grows as the project evolves.
- If Part A and Part B ever appear to disagree, Part B wins on technical
  specifics (it carries the detailed reasoning), and Part A wins on
  philosophy and pillars.
- Part B closes with **Open Decisions**, listing what is resolved and what
  still needs a human call, and with **Part 8**, which traces every
  philosophy in Part A to the decisions that serve it and names the places
  where a decision knowingly trades against one.

This document supersedes two earlier, separate drafts: the original
Constitution (which specified Tauri and Preact for the UI) and a later note
exploring Qt Widgets. Both UI directions were superseded by **GPUI plus
gpui-component** after evaluation, documented in Part B, Part 0. Nothing
here is silently rewritten: superseded choices are marked as such, with the
reasoning kept, rather than deleted.

### Revision log

- **2026-11-15 — Ecosystem re-verification.** Every technology choice was
  re-checked against live sources (crates.io, GitHub, release notes) because
  the first draft was written long before implementation started. Result:
  no core choice was overturned (GPUI + gpui-component, sqlx, Rust core), but
  many specifics changed. Sections touched are marked
  **(revised 2026-11)**. Summary of what changed:
  - GPUI is now consumed from crates.io with exact pins (via the `gpui-pre`
    snapshot that gpui-component requires), not from git commits (0.1).
  - gpui-component moved to `longbridge/gpui-kit`; Dock is confirmed, Tiles was
    not found in 0.9.x (0.4, 6.3).
  - Multi-cursor now exists upstream; OPEN-6 shrinks to finishing touches.
  - sqlx moved to 0.9: dynamic SQL needs `AssertSqlSafe`, TLS feature naming
    changed, and cancel-safety bugs mean a cancelled connection is discarded
    (1.2, 3.2; refined by the spike entry below).
  - Each editor tab gets a dedicated session, not a pooled one (2.2, 9.1).
  - SQL handling is a hybrid: an in-house tokenizer for splitting, parameters
    and cursor position, `sqlparser` for ASTs (4.2, 9.3, new 2.7).
  - keyring 4 split into `keyring-core` plus per-platform stores (5.1);
    `directories` is replaced by `etcetera` (5.2).
  - A week-one spike was added as a gate before building the real UI (Part 10).
- **2026-11 — Week-one spike, items 6-8** (live Postgres 17 / MySQL 8.4,
  sqlx 0.9.0). Sections marked **spike** or **revised after spike**:
  - Cancel: explicit side-connection cancel, then drain; a session is kept after
    a clean cancel (health-checked) and closed only on unknown protocol state.
    This reverses "discard after every cancel" (3.2).
  - Editor sessions are opened outside the pool (2.2, 9.1).
  - Ad-hoc SQL runs one statement per `raw_sql` call over the text protocol
    (1.2, 2.3, 2.5, 9.2).
  - The in-house scanner is the only lexer; `sqlparser`'s tokenizer is a test
    oracle (2.7).
  - Keyring wiring decided for Linux; TLS verify modes still to verify (5.1, 5.3).
- **2026-11 — Week-one spike, items 1-5 (GUI)** (X11, AMD GPU,
  gpui-component 0.9.1). Sections marked **(revised 2026-11, spike)**:
  - The Table virtualizes rows and is kept; grid frame-budget rules, loading
    cells, and Core-driven export and sort (6.3, 3.3).
  - Editor used as provided; `tree-sitter-sql` feature and a direct
    `gpui-base` pin in `ui` only; large-file warning; memory is a watch item
    (0.1, 0.4, 6.1).
  - Dock: no Tiles, floating or detach. Result tabs stay in the bottom dock;
    detached windows are Later (0.4, 9.2). Layout persists as an opaque JSON
    file per workspace (5.2).
  - Gate passed for starting the real UI, with scheduled checks left (Part 10).
- **2026-11 — Human testing on X11/MATE.** Sections marked
  **(revised 2026-11, human test)**:
  - Grid end-of-list check passed; Latin input, dead keys and paste work;
    CJK and Wayland still pending (Part 10).
  - The editor must take focus on open and tab activation (6.1); shortcut
    tests go through the real keymap (7.1).
  - Emoji hotkeys depend on an XIM input method; `Ctrl+.` stays bound to
    upstream code actions (VS Code-consistent), and a DBased emoji/symbol
    picker is low priority, command palette only (6.1).
- **2026-11 — Editor UX and Later features.** Sections marked
  **(revised 2026-11, editor UX)** or **(new 2026-11)**:
  - The default keymap preset is VS Code-like (VS Code Windows mapping on
    every OS, confirmed), with desktop-grab notes and the upstream gap list
    (9.4, OPEN-6).
  - Smart multi-cursor, syntax-aware, on by default; the scanner will move to
    a shared pure crate (9.4.1, 1.3, 2.7).
  - Fragment debugging grows into a step-by-step debug mode with a block
    tree, per-step row counts and read-only safety (9.5).
  - New: SQL formatting with portable, deterministic style files (9.6).
  - New: explain visualization over a common plan model, with rollback and
    confirmation for `EXPLAIN ANALYZE` of non-reads (9.7).

---

## Quick reference

| | |
|---|---|
| **Vision** | The SQL client developers enjoy using every day. Best experience, not most features. |
| **Language** | Rust (stable, pinned by `rust-toolchain.toml`), workspace of `commands`, `core`, `ui`, `app` |
| **UI** | GPUI + gpui-component. Native, GPU-accelerated, no webview. Contained to the `ui` crate. Exact-pinned from crates.io. |
| **Async** | `tokio`, confined to `core` |
| **Database access** | `sqlx` 0.9 (runtime API, `AssertSqlSafe` for user SQL), PostgreSQL + MySQL for MVP |
| **SQL handling** | In-house tokenizer (split, parameters, cursor) + `sqlparser` (AST, DDL classification) |
| **Config storage** | TOML (human-facing) + SQLite via sqlx (history) + OS keychain via `keyring-core` stores (credentials) |
| **Result loading** | Ad-hoc queries: bounded buffer in Core. Table browse: keyset pagination, continuous scroll. |
| **Editing** | Table-browse only. Tiered key resolution (PK/unique → read-only → explicit all-columns opt-in). |
| **Docking / Grid / Editor** | gpui-component's Dock / virtualized Table / native code editor (multi-cursor exists upstream; Ctrl+D family built in-house, upstream-first) |
| **Status** | See Part B, Open Decisions, and Part 10 (week-one spike). |

---

## Table of contents

**Part A — Project Constitution**
Overview · Vision · Core Pillars · Product Philosophy · Project Organization ·
Dependency Direction · Core Independence · Own Components · Technological
Independence · MVP Scope · Technology Stack · High Level Architecture ·
Workspace Model · Communication Model · Jobs · Metadata · Result Loading ·
Drivers · Explorer · UX Guidelines · Command Palette · Search · Performance
Goals · Logging · Configuration · Future Features · Code Style · Naming ·
Functions · Readability · Comments · Error Handling · Architecture
Principles · Dependencies · Testing · Commits · Commit Frequency · Pull
Requests · Session Handoff · Refactoring · Development Behavior · Final
Principle

**Part B — Architecture Decisions**
Part 0, UI Platform · Part 1, Foundations · Part 2, The Data Path · Part 3,
Jobs, Cancellation, Errors · Part 4, Metadata and Explorer · Part 5,
Persistence and Security · Part 6, Frontend (GPUI) · Part 7, Testing ·
Part 9, Editor and Session UX · Part 10, Week-One Spike · Open Decisions ·
Part 8, Constitution and Philosophy Alignment

---


# Part A — Project Constitution

> Revision note. Earlier drafts of this Constitution specified Tauri and
> Preact for the UI layer. A later note explored Qt Widgets. Both were
> superseded by **GPUI (the Zed framework) plus gpui-component (Longbridge)**
> after evaluation. The reasoning, alternatives considered, and containment
> strategy live in Part B, Part 0. This document reflects the current
> decision throughout. Sections that changed as a result are marked
> **(updated)**; nothing else was altered. Sections changed by the
> 2026-11 re-verification are marked **(revised 2026-11)**.

## Overview

You are helping build **DBased**, a modern desktop SQL client focused on **performance, simplicity and user experience**.

This document is the project's constitution.

Every architectural, design and implementation decision must follow these principles.

When in doubt, follow this document over assumptions. Where this document and
Part B (Architecture Decisions) appear to disagree on a technical specific,
Part B wins, since it carries the detailed reasoning; this document wins on
philosophy and pillars.

---

# Vision

DBased is **not** intended to become "another DBeaver".

It is intended to become the SQL client that developers actually enjoy using every day.

The goal is not to have the most features.

The goal is to provide the best experience.

---

# Core Pillars

Every decision must improve at least one of these pillars without significantly harming the others.

* Performance
* Simplicity
* User Experience
* Maintainability

If a change does not improve any pillar, it probably should not be part of the project.

---

# Product Philosophy

## Performance First

The application should feel instant.

Users should never wonder whether the application froze.

Loading indicators are acceptable.

Frozen interfaces are not.

---

## Simplicity First

Every feature increases complexity.

New features should only exist if they improve the daily workflow.

Avoid feature bloat.

Avoid unnecessary configuration.

---

## UX First

The common workflow should require as few clicks as possible.

Frequently used actions should always be easy to access.

Keyboard navigation should be treated as a first-class feature.

---

## Hidden Power

Advanced features should exist without making the interface intimidating.

Power should be discoverable, not constantly visible.

---

## No Magic

Code should behave exactly as it appears.

Avoid hidden side effects.

Avoid unexpected automatic behaviors.

Explicit is better than implicit.

---

## Engineering Quality Over Speed

DBased is developed prioritizing engineering quality, architecture, performance,
low resource consumption, and the project's longevity. Speed of development is
not a decision criterion when it would compromise the quality of the solution.

Every technical decision must consider the project's sustainability over the
long term.

---

# Project Organization *(updated)*

The project uses a **Cargo workspace (monorepo)**. Each crate has a single
responsibility and explicit dependencies. New crates are created only when
there is a clear architectural reason.

The current crate layout is: `commands`, `core`, `ui`, `app` (package names
`dbased-commands`, `dbased-core`, `dbased-ui`, `dbased-app`). The concrete
structure, what belongs in each crate, and the reasoning are in Part B, 1.3.

---

# Dependency Direction *(updated)*

Dependencies must always point toward the Core.

```
App

↓

UI

↓

Commands

↓

Core

↓

Drivers
```

No internal module may depend on the UI.

Drivers must never know about the UI.

The Core must never import any UI-framework library.

---

# Core Independence *(updated)*

The Rust Core must never depend on:

* GPUI
* gpui-component
* Any other UI framework

The Core should be reusable by:

* CLI
* another GUI
* server
* tests

without modification.

---

# Own Components

Whenever possible, the components that define DBased's identity should be
built by the project itself.

Examples:

* Data Grid
* Connection Tree
* SQL Editor
* Command Palette
* Docking Manager

Generic components, such as menus, dialogs, toolbars, splitters, and status
bar, may use what the UI framework provides directly.

---

# Technological Independence

The Core is the project's greatest asset.

The UI must be treated as a replaceable presentation layer.

All business logic, connection management, query execution, cache, history,
plugins, commands, and other functionality must remain independent of
whichever UI technology is in use.

---

# MVP Scope

The first version should support only:

* PostgreSQL
* MySQL

Nothing else.

The MVP must provide:

* Connection management
* Schema explorer
* Tables
* Views
* Functions
* Procedures
* SQL editor
* Query execution
* Query cancellation
* Result visualization
* Simple row editing
* CSV export

Anything beyond this belongs to future milestones. Detailed elaboration of the
SQL editor and query execution items (tabs, multi-statement scripts, execution
modes, parameters, keymaps) is in Part B, Part 9.

---

# Technology Stack *(updated, revised 2026-11)*

## Core

Rust

Responsible for:

* connections
* query execution
* metadata
* cache
* drivers
* configuration
* logging
* jobs
* background jobs

All business logic belongs here.

---

## UI

**GPUI** (the Zed framework) plus **gpui-component** (Longbridge, now
`longbridge/gpui-kit`), consumed from crates.io with exact version pins.

Native, GPU-accelerated desktop UI. No webview, no separate frontend runtime.
Desktop-first. No web version is planned.

This is a deliberate high-ceiling, lower-maturity bet: GPUI is pre-1.0 and
gpui-component depends on a snapshot crate (`gpui-pre`) published by its own
maintainer. The bet is contained to the `ui` crate so it stays reversible; the
Core is unaffected regardless of how the UI evolves. Full reasoning,
alternatives considered (Qt Widgets, Slint, egui, Iced, Floem, Xilem, Freya,
Dioxus, a Tauri-plus-web hybrid), and the containment strategy are in Part B,
Part 0.

---

## SQL Editor

gpui-component's native code editor (Tree-sitter highlighting, LSP support,
multi-cursor basics), extended in-house where needed (select-next-occurrence,
select-all-occurrences) and with configurable keymap presets. No embedded
webview editor. Details in Part B, 6.1 and the resolved OPEN-6.

---

## Data Grid

gpui-component's virtualized Table, fed by windowed fetches from the Core.

---

## Docking

gpui-component's Dock, for panel arrangement, resizing, and layout
persistence.

---

## Virtualization

Every large collection must be virtualized.

Examples:

* query results
* explorer tree
* lists
* history

Never render thousands of components. Provided natively by gpui-component's
virtualized Table and List.

---

## Database Access

`sqlx` 0.9, async, using its runtime query API rather than compile-time
macros, covering PostgreSQL and MySQL through one dependency behind the
project's own `Driver` trait.

---

## Async Runtime

`tokio`, confined to the Core. The UI event loop and the Core's async runtime
are reconciled through an explicit seam. Details in Part B, 1.1 and 0.3.

---

# High Level Architecture *(updated)*

The architecture must remain layered.

```
GPUI UI

↓

Commands / Events

↓

Rust Core

↓

Database Driver

↓

Database
```

The UI and Core now share a process, so this boundary is an in-process typed
Rust API rather than a serialized IPC channel, but it is kept just as strict:
the UI never calls Core internals directly, and never communicates directly
with database drivers. Details in Part B, 0.2.

---

# Workspace Model

Everything belongs to a Workspace.

```
Workspace

├── Connections
│
├── Tabs
│
├── Layout
│
├── History
│
└── Settings
```

Tabs belong to connections.

Connections belong to workspaces.

Avoid global mutable state.

---

# Communication Model

Communication should be event-driven.

Example:

```
ExecuteQuery

↓

QueryStarted

↓

RowsReceived

↓

RowsReceived

↓

RowsReceived

↓

QueryFinished
```

Everything should work similarly.

Examples:

* OpenConnection
* CloseConnection
* RefreshMetadata
* CancelQuery
* ExportCsv

The UI sends commands.

The Core emits events.

---

# Jobs

Every long-running operation should become a Job.

Examples:

* query execution
* CSV export
* metadata refresh
* future imports
* future backups

Jobs expose state:

* queued
* running
* completed
* failed
* cancelled

---

# Metadata

Metadata is managed exclusively by the Core.

The UI never decides:

* caching
* invalidation
* refresh strategy

Metadata includes:

* schemas
* tables
* views
* columns
* indexes
* constraints
* foreign keys

---

# Result Loading

Never load an entire result set.

Flow:

```
Query

↓

Cursor

↓

Rust Cache

↓

UI
```

The UI requests pages.

Example:

0–200

201–400

401–600

Never attempt to load millions of rows.

---

# Drivers

Drivers should implement a common interface.

The Query Engine should not know which database it is using.

Avoid giant match expressions.

Prefer traits and composition.

---

# Explorer

Metadata should be loaded lazily.

Expanding a node loads only its children.

Connecting to a database must not load the entire metadata tree.

---

# UX Guidelines

The application should resemble a modern desktop application.

Reference:

VS Code

Avoid:

Eclipse-style interfaces.

Goals:

* clean layout
* generous spacing
* minimal icons
* keyboard-first navigation
* responsive interactions

---

# Command Palette

Implement early.

Every major action should eventually be accessible from it.

Examples:

* Open Connection
* Run Query
* Search Table
* Refresh Metadata
* Toggle Theme
* Open Settings

---

# Search

Searching should be global.

Users should not need to manually expand large trees.

Search should eventually include:

* schemas
* tables
* views
* columns
* functions

---

# Performance Goals

Startup:

< 1 second

UI:

Never blocks

Scrolling:

Always smooth

Memory:

Keep usage low

Measure before optimizing.

Optimize architecture first.

Micro-optimizations come last.

---

# Logging

Structured logs.

Examples:

* connection
* query
* metadata
* ipc
* cache
* errors

Logs should contain context. Secrets (passwords, tokens, key material) never
appear in logs or `Debug` output.

---

# Configuration

Human-readable configuration.

Prefer:

TOML

or

JSON

Concrete storage split (TOML for human-facing config, SQLite for
machine-managed history, OS keychain for credentials) is in Part B, 5.1
and 5.2.

---

# Future Features

Not part of the MVP.

Examples:

* SSH
* Explain Visual
* ERD
* Plugins
* AI Assistant
* Git integration
* Snippets
* Sync
* Themes
* Import tools
* Schema diff
* Migration tools

These should not influence the MVP architecture negatively.

---

# Code Style

Everything must be written in English.

Including:

* files
* folders
* variables
* functions
* structs
* enums
* traits
* commits
* documentation
* comments

Do not mix languages.

---

# Naming

Prefer readable names.

Good:

connection

connectionId

selectedTable

databaseDriver

Bad:

conn

db

tbl

tmp

res

Clarity is always preferred over brevity.

---

# Functions

Functions should do one thing.

Split large functions.

Avoid files with thousands of lines.

---

# Readability

Readable code is preferred over clever code.

Avoid:

* nested one-liners
* unnecessarily long iterator chains
* deeply nested logic
* difficult-to-read expressions

Prefer:

* intermediate variables
* early returns
* small functions
* explicit control flow

---

# Comments

Comments explain WHY.

Never explain WHAT.

Use comments sparingly.

Accepted formats:

```rust
// @TODO: ...

// @FIXME: ...

// @NOTE: ...
```

Good code should be mostly self-documenting.

---

# Error Handling

Never panic in production.

Avoid:

unwrap()

expect()

outside tests.

Return meaningful errors. The workspace enforces this with clippy lints
(`unwrap_used`, `expect_used`) and forbids `unsafe` code.

---

# Architecture Principles

Prefer:

* composition
* traits
* dependency injection
* immutable state

Avoid:

* singletons
* global mutable state
* tight coupling

---

# Dependencies

Every dependency has a cost.

Before adding one, ask:

* Can the standard library solve this?
* Is this dependency actively maintained?
* Is the dependency worth the maintenance cost?

Prefer fewer dependencies.

A dependency is declared in a crate only when that crate uses it.

---

# Testing

Business logic should be testable independently.

The Rust Core must be testable without any UI.

---

# Commits

Use Conventional Commits.

Examples:

```
feat(connection): add postgres connector

feat(query): implement query cancellation

fix(metadata): avoid duplicated schemas

refactor(cache): simplify invalidation

docs(architecture): describe event flow

test(query): add pagination tests
```

---

# Commit Frequency

Commit early.

Commit often.

Do not wait until an entire module is complete.

Each commit should represent one logical change.

Avoid large commits.

Protect progress continuously.

---

# Pull Requests

One feature per pull request.

Avoid unrelated changes.

---

# Session Handoff *(added)*

Work moves between machines and between AI sessions.

The repository, not the conversation, is the source of continuity.

Every working branch carries a `HANDOFF.md` at the repository root.

Rules:

* Update it as the work progresses, not only at the end of a session.
* Update it at every meaningful milestone, and always before ending a session.
* Record: what is done, what is in progress, the next concrete step, and open questions.
* Keep it short and current. Overwrite stale content instead of appending a log.
* Commit it with the work and push it, so another machine can pull and continue.
* Write it in English, like everything else in the project.
* A new session starts by reading this document and `HANDOFF.md`.

`HANDOFF.md` is working state, not documentation.

It never reaches the master branch.

Delete it in the final squash, the one that prepares the branch for merge, and confirm that the pull request diff does not contain it.

Durable decisions never live only in `HANDOFF.md`.

Anything that must outlive the branch belongs in this document (Part B) or in the code documentation.

---

# Refactoring

Do not intentionally accumulate technical debt.

If a feature makes the code noticeably harder to understand, refactor before moving forward.

Always leave the project cleaner than you found it.

---

# Development Behavior

When implementing features:

* Prefer maintainability over speed of implementation.
* If a requirement is ambiguous, choose the simplest solution that respects this constitution.
* Do not introduce unnecessary abstractions.
* Avoid overengineering.
* Build only what is required for the current milestone.
* Design today's code so tomorrow's features fit naturally.

Whenever proposing a significant architectural change, explain:

* why it improves the project;
* which pillar(s) it benefits;
* what trade-offs it introduces.

---

# Final Principle

DBased should feel like a native, modern, lightweight tool.

Every interaction should communicate one idea:

**Fast. Clean. Predictable.**

---

# Part B — Architecture Decisions

Companion to Part A, the Constitution. The Constitution states *principles*.
This part states *decisions*: the concrete, locked choices that the
Constitution leaves open and that are expensive to change once code exists.

Format of each decision:

- **Decision**: what we commit to.
- **Why**: the reasoning and which pillar it serves.
- **Trade-off**: what we give up.

When a decision conflicts with an assumption in code, this document wins on
technical specifics; the Constitution wins on philosophy and pillars.
Decisions marked **OPEN** in the Open Decisions section need a human call
before the related code is written.

---

## Part 0: UI Platform (decided, revised 2026-11)

**Decision.** The UI is built with **GPUI** (the Zed framework) plus
**gpui-component** (Longbridge; the repository is now `longbridge/gpui-kit`).
This is a deliberate high-ceiling, lower-maturity bet. The bet is accepted
knowingly, on the condition that its blast radius is contained to the UI layer
(0.1).

**Re-verification result (2026-11-15).** GPUI + gpui-component was compared
again against Slint, egui (+ egui_table / egui_dock), Iced, Xilem/Masonry,
Floem, Dioxus (native/Blitz), Freya, Tauri 2, and Qt via cxx-qt. Among native,
non-webview stacks only GPUI + gpui-component ships all of: a virtualized
table, Dock, a tree-sitter editor with LSP hooks, a command palette, and
AccessKit accessibility. Slint, egui and Iced would each require building the
grid, dock or editor from scratch; Xilem and Floem are stalled; Freya is a
single-maintainer project without a proven grid/dock/editor. Tauri (webview,
best ecosystem) and Qt (most mature widgets, weak Rust ergonomics) are the
only credible alternatives, each giving up one core requirement. Decision:
keep GPUI + gpui-component. Closest real-world analogs built on the same
stack: dbdelve and dbflux (both active as of this date).

### 0.1 Containment: the bet must be reversible *(revised 2026-11)*

**Decision.** GPUI and gpui-component are allowed **only** inside the `ui` crate.
The `core` crate never imports `gpui`, `gpui-component`, or any of their types.

Dependency sourcing: gpui-component is now published on crates.io and requires
`gpui-pre` (a snapshot of Zed's `gpui`, published by the gpui-component
maintainer, **not** by Zed). The official `gpui` crate on crates.io is a stale
snapshot from October 2025 and must not be mixed in. Therefore:

- every `gpui-*` crate is pinned with an exact `=x.y.z` requirement and
  recorded in the workspace manifest;
- upgrades are deliberate and scheduled (roughly monthly, or when a needed fix
  lands), never automatic, and happen on their own branch;
- *(Spike, 2026-11.)* The UI also needs two things the default set lacks:
  gpui-component's `tree-sitter-sql` feature (SQL highlighting) and a direct
  `gpui-base` dependency (the Dock lives there, and the `AddCursorAbove` /
  `AddCursorBelow` actions are not re-exported by `gpui_component::input`).
  Both are declared only in `crates/ui` (added with the first code that uses
  them), `gpui-base` is exact-pinned to the same version as gpui-component and
  upgraded in lockstep with it, and neither ever reaches `core`;
- escape hatches, in order: vendor the pinned crates (`cargo vendor`);
  switch to a git revision of Zed's `gpui` if gpui-component stays compatible;
  the community fork `gpui-ce`; and, as a last resort, a different frontend
  (Tauri or Qt) on the same Core.

**Why.** The accepted risk is "a future rewrite of the UI", not "a rewrite of the
project". That distinction only holds if the toolkit cannot leak past the `ui`
crate. Exact pins contain the instability, since gpui-component ships breaking
changes on minor versions roughly weekly.

**Trade-off.** Some ergonomic friction: the UI cannot reach into Core internals
directly and must go through the command/event API (0.2). Additionally, the
whole UI stack depends on a snapshot maintained by one or two people backed by
one company (bus-factor risk), accepted and mitigated by the escape hatches
above and by the spike in Part 10.

### 0.2 No IPC boundary, but keep the command/event API

**Decision.** With GPUI, UI and Core are Rust in the same process, so there is no
serialization/IPC boundary anymore (Tauri's IPC, and `tauri-specta` type
generation, are gone). Even so, the command/event contract stays a **typed Rust
API**: the UI sends `Command` values and consumes `Event` values through a thin
boundary, instead of calling Core functions ad hoc. The transport is now an
in-process channel; it used to be IPC; it could be IPC again under a different UI.

**Why.** Keeping the seam explicit is what lets a future Tauri (or any other) UI
sit on the exact same Core by adding only a thin transport adapter. Drop the seam
and let GPUI entities call Core internals directly, and the reversibility in 0.1
evaporates. The command/event enums survive every UI change; only the transport
differs.

**Trade-off.** A boundary that looks like overkill in a single-process app. It is
cheap insurance for the bet.

### 0.3 Runtime seam (Core tokio ↔ GPUI executor)

**Decision.** Two async worlds coexist. The Core runs database I/O on its own
multi-threaded `tokio` runtime (sqlx needs it). GPUI runs the UI event loop with
its own executor on the main thread. They communicate by **bounded** channels
(`tokio::sync::mpsc`): Core pushes progress/results onto a channel; the UI
drains it using GPUI's background executor and applies changes to UI state by
scheduling a closure on the GPUI event loop. Latest-state values (for example
connection status) use `tokio::sync::watch`. The UI thread never blocks on
database work. (Exact GPUI APIs may shift while it is pre-1.0; the pattern,
tokio for I/O plus event-loop hand-off for UI updates, is the stable part.)

**Why.** Replaces the Tauri Channel mechanism. Keeps the "UI never blocks" pillar
intact across the new toolkit and keeps tokio (and therefore sqlx) entirely inside
Core. Bounded channels give natural backpressure.

**Trade-off.** Two executors to reason about. Standard for a native UI plus an
async I/O core.

### 0.4 Superseded Tauri-era UI decisions *(revised 2026-11, spike)*

These earlier decisions are replaced by their gpui-component equivalents. The
Core-side decisions they depended on are unaffected.

- **SQL editor:** Monaco (and its lazy-load, see 6.1) is replaced by
  gpui-component's native code editor (Tree-sitter highlighting, line numbers,
  find, soft wrap, tab config, LSP-capable). Its own docs target about 50K
  lines; the spike opened 100K lines acceptably (Part 10, item 2). No webview,
  so the startup-budget concern no longer applies. Honest
  scope: this editor is *sufficient* for a SQL client, not *equivalent* to
  Monaco. Rich find-and-replace, minimap, code folding, snippets and a VS Code
  keymap are not provided. **Multi-cursor now exists upstream** (Alt-click,
  add-cursor-above/below, and a public `add_selection` API merged in October
  2026); see OPEN-6. Contained to `ui`.
- **Data grid:** TanStack Table/Virtual is replaced by gpui-component's
  virtualized `Table`. Its rows are virtual (the delegate reports a row count
  and renders cells on demand; only visible rows are requested), verified at
  5,000,000 logical rows. The grid is still always fed by windowed fetches from
  the Core (2.3, 2.4): cell access is synchronous, so rows must already be in a
  small UI-side window cache when painted (6.3).
- **UI state:** Zustand is replaced by GPUI entities. The boundary in 6.2
  (bulk result rows stay out of global app state) still holds: large row buffers
  live in Core (2.3), not in UI entities.
- **Docking:** gpui-component's **Dock** (in `gpui-base`; resizable panels,
  draggable tabs, nested splits, serializable layout). Placements are only
  left, right, bottom and center: there are **no Tiles and no floating,
  popup or detached panels** in 0.9.x, and none are planned for the MVP.
  Layout persists per workspace (5.2, 6.3).
- **IPC type generation:** `tauri-specta` is not used; 0.2 replaces it.

### 0.5 What this change does NOT touch

Everything in Core survives the UI swap unchanged, which is the whole point and the
reason the bet is affordable: the sqlx layer (1.2), the Driver trait (2.1), the
pool and session model (2.2), result loading and the row buffer (2.3, 2.4), the
Value model (2.5), editing (2.6), jobs and cancellation (3.1, 3.2), export (3.3),
the error model (3.4), metadata caching and DDL invalidation (4.1, 4.2),
credential storage and workspace persistence (5.1, 5.2), and the testing
strategy (Part 7). If GPUI is abandoned in a year, this is the part that does
not get rewritten.

---

## Part 1 — Foundations

### 1.1 Async runtime

**Decision.** The Core uses a single `tokio` multi-threaded runtime for all
database I/O. The app starts this runtime; GPUI runs the UI event loop separately
(see 0.3 for the seam between them). The Core exposes an async API and stays
runtime-agnostic at its boundary. CPU-bound work (CSV serialization, large row
encoding) runs on `tokio::task::spawn_blocking` or a dedicated thread, never on
the I/O reactor. Cancellation uses `tokio_util::sync::CancellationToken`.

**Why.** Keeping tokio (and sqlx) entirely inside Core, behind a runtime-agnostic
API, preserves Core Independence: a CLI, test harness, or future non-GPUI UI
provides its own way to drive it. Serves Performance and Maintainability.

**Trade-off.** Two executors coexist (Core tokio + GPUI). Reconciled in 0.3. The
Core API is async end to end, so synchronous test code must spin up a runtime.
Acceptable.

### 1.2 Database access crate *(revised 2026-11, spike)*

**Decision.** Use `sqlx` 0.9 (MSRV 1.94) with the runtime query API, **not** the
`query!` compile-time macros. Features: `runtime-tokio`, an explicit rustls
variant with native roots (for example `tls-rustls-ring-native-roots`; the bare
`tls-rustls` is only a compatibility alias), `postgres`, `mysql`, `sqlite`. No
`DATABASE_URL` at build time. Concrete rules that follow from sqlx 0.9 and from
running arbitrary SQL:

1. **User SQL is wrapped in `AssertSqlSafe`.** sqlx 0.9 requires `&'static str`
   or `AssertSqlSafe(..)` for every query function. A SQL client executes
   user-written SQL by definition, so the wrapper is applied at exactly one
   place in each driver (the execution entry point), never scattered. The
   invariant is enforced by types, not by comments: a `UserSql` newtype
   represents SQL typed by the user, and a single conversion function per
   driver turns `UserSql` into `AssertSqlSafe`. DBased's own statements
   (metadata, pagination, editing) stay `&'static str` or are assembled with
   `quote_identifier` and never become `UserSql`. The only comment lives on
   that conversion function and is short and version-independent (why user SQL
   is executed as-is, and that SQL we assemble ourselves must never pass
   through it).
2. **Ad-hoc SQL uses `raw_sql`, one statement per call** (text protocol on
   both databases, so every value arrives as the server renders it, see 2.5).
   Scripts are split by our tokenizer and sent statement by statement (9.2),
   read with `fetch_many` (rows, then one `QueryResult` boundary). `sqlx::query`
   is never used for unparameterized user SQL: on MySQL it still uses the
   binary protocol, which turns zero-dates into NULL-lookalikes, and it rejects
   multi-statement text. Parameterized statements (9.3) use the extended
   protocol with `persistent(false)` / statement cache disabled, so cached
   plans never go stale after DDL.
3. **Never use `sqlx::Any`.** Its decoding is lossy (decimal, uuid, enums,
   chrono). Use `PgPool` / `MySqlPool` concretely behind the `Driver` trait.
   Raw cell access in 0.9 (`ValueRef::as_bytes()` is private) is
   `try_get_unchecked::<Option<String>, _>` for Postgres text-format values and
   `try_get_unchecked::<Option<&[u8]>, _>` for MySQL and binary-format values,
   with `type_info().name()` choosing the `Value` variant (2.5). Typed
   `try_get::<String>` is not an option: it rejects non-text types.
4. **Never drop a sqlx future or stream to stop work** (it does not stop the
   server, see 3.2). Editor sessions never enter the pool (2.2), and a pool
   connection whose work was cancelled is closed, never returned (3.2).
5. `Cargo.lock` is committed by this project; sqlx itself no longer tracks one,
   so its transitive dependencies drift unless we pin them.

**Why.** A SQL client runs arbitrary user SQL, so compile-time query checking
is useless and the macros would force a live database at build time. sqlx gives,
for Postgres, MySQL and SQLite in one dependency: async pooling, lazy row
streaming with on-demand decoding, and a unified type layer. Re-verification
compared it with `tokio-postgres` + `mysql_async` (+ deadpool), diesel-async and
sea-orm. No alternative covers both databases in one API; the ORMs add nothing
for arbitrary SQL. Serves Maintainability and the "fewer dependencies" rule.

**Trade-off.** sqlx exposes neither a cancel token nor the backend PID, and has
open cancel-safety issues (cancelled `begin()` / `prepare`). The spike showed
a dropped query future does not cancel anything: the server keeps running and
the connection's next use silently waits for it, and the pool does not stop a
stateful connection from being reused. So cancellation is done at the SQL
layer from a side connection, and connection hygiene is our policy (3.2). sqlx
also releases slowly (0.8.6 in May 2025, 0.9.0 in May 2026) and fixes sit on
`main` for months; we pin the version and accept taking a git revision if
needed. **Escape hatch:** because every database sits behind the `Driver` trait,
the Postgres driver alone can be reimplemented on `tokio-postgres` (native
`CancelToken` with a real wire CancelRequest, `simple_query`, `connect_raw` for
SSH streams, portals for batched fetch) without touching the engine. We start
on sqlx and switch only if cancel-safety or streaming problems bite in practice.

### 1.3 Cargo workspace layout *(revised 2026-11)*

**Decision.** An all-Rust Cargo workspace, four crates, dependencies pointing
toward the Core:

```
dbased/
└── crates/
    ├── commands/   # dbased-commands: the contract. Command / Event enums
    │                 and shared DTOs (Value, ColumnMeta, ConnectionConfig,
    │                 JobId, error shape). No dependency on core or gpui.
    ├── core/       # dbased-core: business logic: sqlx, Driver trait + driver
    │                 impls (as modules, e.g. drivers::postgres,
    │                 drivers::mysql), jobs, cache, metadata, SQL tokenizer,
    │                 workspace/settings persistence, credential storage.
    │                 No GPUI. Depends on `commands`.
    ├── ui/         # dbased-ui: GPUI + gpui-component: panels, editor, grid,
    │                 dock, entities (UI state). Depends only on `commands`,
    │                 never on `core` directly (see 0.2).
    └── app/        # dbased-app: binary `dbased`. Depends on `ui` and `core`.
                      Starts the tokio runtime and the GPUI event loop, and
                      wires Core's command executor to the UI's command/event
                      channel (the seam in 0.3).
```

`core` and `ui` both depend on `commands`; `ui` does not depend on `core`;
`app` depends on `ui` and `core`. Dependency direction: `app → ui → commands`
and `app → core → commands`, so nothing points away from the Core toward the
UI. Drivers stay as modules inside `core` until a concrete architectural need
(for example a plugin system) justifies splitting them. *(Revised 2026-11,
editor UX.)* One such split is planned: the SQL scanner (2.7) moves into a
pure `dbased-sql` crate, used by `core` and `ui`, when the first synchronous
editor feature needs it (9.4.1). Approved by the product owner (2026-11).

Workspace-wide enforcement (in the root manifest): `unsafe` code is forbidden,
clippy denies `unwrap_used` and `expect_used`, `[workspace.dependencies]`
centralizes versions, and the toolchain is pinned by `rust-toolchain.toml`.
The dependency rules are verified with `cargo tree` (core has no `gpui`, ui has
no `dbased-core`).

**Why.** Enforces Core Independence physically: the compiler refuses a `gpui`
import inside `core`, and refuses a `core` import inside `ui`, since `ui`
only ever sees the `commands` contract. That second constraint is stricter
than strictly necessary for Core Independence alone, but it is what keeps the
command/event boundary (0.2) real instead of aspirational. Serves
Maintainability and Testing.

**Trade-off.** Slightly more boilerplate than a single crate, and everything
Core wants to expose to the UI must be expressed as a `commands` type.
Worth it for the guarantee.

### 1.4 Contract type safety

**Decision.** Because `ui`, `core`, and `app` are all Rust in one workspace,
the `commands` crate's `Command` and `Event` enums are used directly by both
sides. No serialization boundary, no code generation step, no bindings to keep
in sync.

**Why.** This is a direct win from the GPUI decision (Part 0) over the earlier
Tauri-era plan. A backend rename is now a compiler error on the `ui` side
directly, with zero tooling. Serves Maintainability and "No Magic".

**Trade-off.** None; this is strictly simpler than the IPC-era approach it
replaces.

---

## Part 2 — The Data Path

This is the heart of the application and where the Constitution was thinnest.

### 2.1 The Driver trait *(revised 2026-11, spike)*

**Decision.** One trait abstracts the database. The Query Engine talks only to
this trait. Dialect differences live behind it, not in `match` expressions.
`async-trait` stays: native `async fn` in traits is stable but not
dyn-compatible, and the engine holds drivers as trait objects. (An enum of
backends would avoid the macro, but a trait object fits the extension story
better; the per-call boxing is irrelevant next to a network round trip.)

```rust
#[async_trait]
pub trait Driver: Send + Sync {
    /// Opens a driver for a saved connection. A free constructor per driver
    /// (not part of the trait object) keeps `dyn Driver` usable.
    // connect lives on a separate `DriverFactory` trait.

    /// Opens a dedicated session (one physical connection, outside the
    /// pool) for an editor tab. See 2.2.
    async fn open_session(&self) -> Result<Box<dyn Session>, CoreError>;

    // Metadata, loaded lazily per node (see Part 4).
    async fn list_schemas(&self) -> Result<Vec<Schema>, CoreError>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableRef>, CoreError>;
    async fn list_columns(&self, table: &TableRef) -> Result<Vec<Column>, CoreError>;
    async fn list_views(&self, schema: &str) -> Result<Vec<ViewRef>, CoreError>;
    async fn list_routines(&self, schema: &str) -> Result<Vec<Routine>, CoreError>;

    // Dialect helpers, kept tiny and pure.
    fn quote_identifier(&self, ident: &str) -> String;
    fn paginate(&self, base_sql: &str, page: PageRequest) -> String;
    fn rewrite_parameters(&self, tokens: &[Token]) -> RewrittenSql;
}

#[async_trait]
pub trait Session: Send {
    /// Backend identifier captured when the session is opened
    /// (`pg_backend_pid()` i32 / `CONNECTION_ID()` u64, stored as u64),
    /// used by cancellation.
    fn session_id(&self) -> SessionId;

    /// Runs ONE statement and streams its rows into the sink. Column metadata
    /// arrives with the first row, or from a describe when there are none
    /// (2.3). On cancel it drains to the terminal result; it never returns
    /// with the statement still in flight (3.2).
    async fn execute(
        &mut self,
        sql: UserSql,
        sink: RowSink,
        token: CancellationToken,
    ) -> Result<StatementOutcome, CoreError>;

    /// Cheap liveness probe (`SELECT 1`) used after a cancel. See 3.2.
    async fn health_check(&mut self) -> Result<SessionHealth, CoreError>;
}

#[async_trait]
pub trait Canceller: Send + Sync {
    /// Issues a cancel from a SEPARATE connection. See 3.2.
    async fn cancel(&self, session: SessionId) -> Result<(), CoreError>;
}
```

(The scaffold currently carries the first draft of this trait; the Session /
Canceller split is applied when the first real driver is written.)

**Why.** Matches the Constitution's "traits and composition, not giant match".
Postgres and MySQL each implement it once. Adding a third database later is a
new impl, not a change to the engine. Splitting `Session` from `Driver` makes the
per-tab dedicated connection (2.2) a first-class concept instead of a pool
side effect. Serves Maintainability.

**Trade-off.** Some duplication between impls for similar metadata queries. We
accept duplication over a leaky shared base.

### 2.2 Connection, pool and session model *(revised 2026-11, spike)*

**Decision.** Each saved Connection owns one pool (small, e.g. max 5) used for
**stateless** work: metadata loading, table browse, export, and cancellation
side-connections. Each **editor tab owns a dedicated session**: one physical
connection opened directly from the same connect options (**not** checked out
of the pool) on first run and held for the life of the tab (closed when the
tab closes or after a configurable idle timeout). A running query borrows
nothing extra; it runs on its tab's session. The pool is created on first use
("Open Connection"), not at app start. Connect options are built with the
typed sqlx builders, never as URL strings, so the password never sits in a
formatted string.

Sessions stay outside the pool because the spike showed (a) session state
lives on the physical connection and survives a return to the pool, so a
pooled session would leak `SET search_path` / `USE db` / temp tables into
metadata queries, and (b) with sessions held from the pool, a full pool makes
even the cancel side connection fail with `PoolTimedOut`. Out-of-pool sessions
make both impossible by construction rather than by accounting.

**Why.** Users expect session state to persist between executions in the same
editor: `SET`, temporary tables, `USE database`, transactions opened by hand.
Executing each run on an arbitrary pooled connection would silently break all
of that. Keeping stateless work on the pool still guarantees a connection is
never held hostage by a result grid: results are copied into the Core buffer
(2.3) and the session is free for the next statement immediately. We still do
**not** keep server cursors open per tab. Serves UX and Performance.

**Trade-off.** One physical connection per open editor tab on top of the pool,
so the server sees up to pool size plus open tabs; sessions get none of the
pool's lifetime management, so idle timeout and reconnect are ours. The idle
timeout and a clear "session released" indicator keep this bounded and
visible (No Magic). Table-browse pagination (2.4) re-queries per portion on the
pool, an extra round trip per page versus a held cursor; cheap, and it frees
connections.

### 2.3 Result loading: ad-hoc queries *(revised 2026-11, spike)*

**Decision.** Running arbitrary SQL streams rows from `sqlx` into a **bounded**
in-memory buffer inside the Core, then frees the session. The UI paginates
against that Core buffer. The buffer has **two caps, whichever is reached first**:
a maximum row count and a maximum memory budget. Both are **user-configurable**
(defaults: 200,000 rows / 512 MB, treated as starting points to tune). When
either cap is reached, the Core stops buffering, cancels the statement and
drains the session (3.2; simply dropping the stream would let the server keep
producing and stall the next run), marks the result `truncated`, and tells the
UI to suggest refining the query or exporting (export streams to disk without
buffering, see 3.3). sqlx's stream does not buffer client side (memory stayed
flat over 2M rows), so the caps are the only buffer.

Column headers come from the first row. A statement that returns no rows
carries no column metadata through `raw_sql`, so when the statement looks
row-returning the Core describes it (prepare only, nothing executes) to show
headers; a describe failure just shows no headers. The row
count shown is the rows received; `rows_affected` is shown only for
statements without rows (Postgres reports it for `SELECT`, MySQL reports 0).

```
ad-hoc SQL → sqlx stream → Core bounded buffer → session free
                                    ↓
                          UI requests pages (0–200, ...)
```

The grid never receives the full buffer: it asks for the windows around the
visible range and keeps only a small bounded cache of them (6.3). The grid was
verified at 5,000,000 logical rows (Part 10); a row cap configured above that
is untested for scroll precision.

**Why.** Honors "never load millions of rows" while keeping paging instant
(it reads from RAM, not the wire). Serves Performance and Simplicity.

**Trade-off.** The cap means a huge ad-hoc result is shown partially. This is
correct behavior for a desktop client and the truncation is explicit, not
magic.

### 2.4 Result loading: table browse

**Decision.** Browsing a table (double-clicking it in the explorer) is a
**separate mode** from ad-hoc SQL. It uses **keyset pagination** by default,
re-querying per portion, and does not buffer the whole table. The default
portion size is **200 rows, user-configurable** (shared setting with the ad-hoc
page size).

Three points this decision pins down:

1. **Keyset implies continuous scroll, not numbered pages.** Keyset cannot jump
   to an arbitrary page (`row 50,000`) directly; it walks forward from the last
   seen key. So the grid commits to the DBeaver-style model: fetch the next
   portion when the user scrolls to the bottom, accumulating fetched rows so
   scrolling back up reads from memory. "Go to row N" navigates within already
   fetched rows.
2. **Keyset needs a deterministic total order.** It pages on the primary key (or
   a unique index). When the user sorts by a non-unique column, the Core appends
   the unique key as a tiebreaker so the order stays total and the cursor stays
   correct.
3. **No usable unique key means no keyset.** A table with no PK and no unique
   index falls back to `LIMIT/OFFSET` (slower on deep scroll, acceptable because
   such tables are rare and usually small). On Postgres the Core may order by
   `ctid` as a cheaper physical-order fallback within a single browse session.
   This is the same class of table that is read-only-by-default for editing
   (see 2.6).

**Why.** Keyset is the most optimized option and fits the app's performance
philosophy. Serves Performance and UX.

**Trade-off.** Two code paths (ad-hoc vs browse) and a slower fallback for
keyless tables. Both are deliberate and bounded.

### 2.5 The Value model *(revised 2026-11, spike)*

**Decision.** Decode every cell into one common enum. Anything we do not model
explicitly falls back to the database's text representation.

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(String),   // exact; never f64
    Text(String),
    Bytes(Vec<u8>),    // shown as hex / size in grid
    Timestamp(String), // as rendered by the server (DateStyle, session tz)
    Date(String),
    Time(String),
    Uuid(String),
    Json(String),      // raw JSON text, not re-parsed
    Unknown(String),   // driver's text form for unmodeled types
}
```

Column metadata (name, declared type, nullability) travels **once** per result.
Rows travel as arrays of `Value`, not as objects, to avoid repeating keys.

Decoding is per driver, from the raw cell plus the driver's type name (1.2,
rule 3). Ad-hoc results use the text protocol, verified faithful in the
spike: Postgres renders every tested type exactly (numeric, timestamptz,
interval, ranges, arrays, enums, composites, jsonb, uuid, inet, money, xml,
geometric, infinity; `bytea` as `\x` hex), and MySQL renders everything
except `BIT`, `GEOMETRY`, `BLOB` / `VARBINARY`, which arrive as raw bytes and
become `Bytes` (hex in the grid; `BIT` also as a bit string). Mapping text to
a typed variant is an optimization: a text that does not fit its variant (for
example `BIGINT UNSIGNED` above `i64::MAX`) falls back to `Unknown`, never
to an error or a rounded value. The Core never rewrites user SQL to fix a
type (no injected `ST_AsText`); table browse, whose SQL we build, may.

Known quirks: Postgres simple mode reports enum, composite and xml type names
as `?`, so the Core resolves unknown type OIDs through a cached `pg_type`
lookup on the pool; MySQL `SET` columns report `CHAR`. Session settings
(`DateStyle`, `bytea_output`, `lc_monetary`) shape Postgres text; DBased shows
what the server rendered and does not normalize it. The binary protocol
(parameterized statements only) needs per-type decoders for numeric,
interval, range, array and composite, strips the jsonb version byte, and must
render MySQL zero-dates faithfully rather than as NULL.

**Why.** A SQL client must never silently corrupt data. `Decimal` and money
types as strings prevent float rounding. `Unknown` guarantees we always render
*something* faithful rather than erroring on an exotic type. Serves UX,
Performance, "No Magic".

**Trade-off.** The frontend formats values for display from typed-but-stringy
data. Acceptable; display formatting is a UI concern anyway.

### 2.6 Editing

**Decision.** In the MVP, **only table-browse results are editable.** Ad-hoc
query results are read-only. Editing resolves the row identity through a
**three-tier key strategy**, following DBeaver's model:

1. **Physical key (default, silent).** If the table has a primary key or a unique
   index, edits use it: `UPDATE <table> SET col = $1 WHERE <key> = $2`, plus the
   analogous `INSERT` / `DELETE`.
2. **No key, read-only by default.** A table without any unique key is read-only
   and the UI says so clearly (a visible indicator, not a silent failure on
   save).
3. **Explicit opt-in: all-columns key.** From the read-only state the user can
   choose "edit using all columns as key". This builds a `WHERE` matching every
   column's current value. It is never automatic; it is an explicit choice with a
   warning that it may match more than one row.

To make tier 3 safe, every all-columns edit runs inside a transaction and checks
the affected-row count: if it is not exactly 1, the Core **rolls back** and
reports it instead of silently changing multiple rows. (A persisted
user-defined "virtual key" on specific columns is the natural post-MVP
extension; not in v1.)

Every edit is a single explicit statement, and the generated SQL is previewable
before it runs.

**Why.** "Simple row editing" is only simple when the row's identity is known. A
joined `SELECT` has no single identity, so ad-hoc results stay read-only. The
tiered fallback matches what users expect from DBeaver while the
rollback-on-multi-row check is a safety net even DBeaver does not enforce.
Serves Simplicity, "No Magic", and data safety.

**Trade-off.** Users cannot edit query results directly in v1, and keyless
tables need a deliberate opt-in. Both are documented limitations, not bugs, and
keep destructive operations honest.

### 2.7 SQL text handling: tokenizer plus parser *(new 2026-11, revised after spike)*

**Decision.** SQL text is processed in two layers, both inside `core` (the
scanner later moves to a pure `dbased-sql` crate shared with `ui`, 1.3, 9.4.1):

1. **An in-house tokenizer** (small, dialect-aware for Postgres and MySQL)
   that never fails on broken SQL. It understands line and block comments
   (including nesting where the dialect allows it), single-quoted and
   double-quoted strings, backtick identifiers, Postgres dollar-quoted strings
   (`$tag$ ... $tag$`), `::` casts, and parameter markers (`:name`, `?`). It is
   the single source for: **statement splitting** (script execution, 9.2),
   **statement under the cursor**, **query parameter detection** (9.3), and
   **block navigation** (9.4). Because it works at token level, half-typed SQL
   still splits and navigates correctly.
2. **`sqlparser`** (Postgres and MySQL dialects) for everything that needs a real
   AST: DDL classification and target extraction for cache invalidation (4.2),
   CTE and subquery extraction (9.5). It is applied only to statements that
   parse; a parse failure is a normal outcome handled by the conservative
   fallbacks in 4.2 and 9.5, never an error shown to the user.

Tree-sitter (via gpui-component's `tree-sitter-sequel` grammar) remains the
editor's syntax-highlighting layer and may later back tolerant, syntax-aware
editor features. It is not used by the Core. `pg_query` (exact Postgres
grammar) is deliberately not adopted: it needs a C toolchain, covers only
Postgres, and rejects partial SQL.

**Why.** `sqlparser` is strict and returns no partial AST for incomplete input,
which makes it the wrong tool for editor-time behavior (cursor, split,
parameters) but the right tool for classification of complete statements.
Splitting responsibilities puts each need on the layer that fits it, and keeps
the "ignore `::`, strings and dollar quotes" requirement in one tested place.
Serves Maintainability and UX.

**Spike outcome (2026-11).** `sqlparser`'s tokenizer (0.64) lexes clean text
correctly but cannot be our base: it returns an error with **no partial
tokens** on an unterminated string, quoted identifier, block comment or
dollar quote (the normal state while typing, and one bad statement loses the
whole script), and its locations count chars, not bytes. The spike suggested
wrapping it for clean text plus a tolerant scanner for broken text; that is
rejected because two lexers would give different splits for the same script
depending on whether a quote is closed yet. The in-house scanner is the
**only** lexer for split, cursor, parameters and navigation; it works in byte
offsets; strings, comments and dollar quotes, the hard part, have to be
tolerant anyway, and the rest is trivial. `sqlparser`'s tokenizer serves as a
**test oracle** (differential tests on complete inputs). AST spans from
`sqlparser` (9.5, error positions) are converted with a per-line char-to-byte
table.

Dialect rules the scanner must implement (confirmed or found by the spike):
nested block comments in Postgres only; `#` comments and backtick
identifiers in MySQL; `E''` strings and `$tag$` quotes in Postgres. To be
covered by the test matrix as well: MySQL `--` needs a following whitespace,
MySQL backslash escapes in strings, MySQL `/*! ... */` executable comments
(treated as comments for splitting, as "may hide DDL" for 4.2). Splitting on
top-level `;` is not enough: `BEGIN ... END` bodies in MySQL `CREATE
PROCEDURE / FUNCTION / TRIGGER / EVENT` and Postgres `BEGIN ATOMIC` bodies
are tracked as blocks (a bare `BEGIN;` is a transaction start), and MySQL's
client-side `DELIMITER` directive is honored by a line-level pre-pass and never
sent to the server.

**Trade-off.** We own and must test a tokenizer. Its scope is intentionally
small (no grammar), and its test matrix (table-driven with `rstest`, snapshot
with `insta`) is the place where DBeaver-style parameter bugs are prevented.
Block tracking for compound bodies is the one piece that edges toward grammar;
it is kept to `BEGIN`/`CASE` ... `END` nesting inside `CREATE` routine
statements (where `END IF`, `END LOOP` and similar close their own
constructs, not a `BEGIN`).

---

## Part 3 — Jobs, Cancellation, Errors

### 3.1 Job system

**Decision.** Every long-running operation (query, export, metadata refresh) is
a `Job` with a `JobId`, a state (`Queued | Running | Completed | Failed |
Cancelled`), and a `CancellationToken`. A `JobRegistry` in Core state holds
active jobs. The UI gets a `JobId` back immediately from the command that starts
the work and listens on a bounded channel keyed by that id.

**Why.** Matches the Constitution's Jobs section exactly and gives one uniform
lifecycle for everything cancellable. Serves Maintainability and UX.

**Trade-off.** None significant.

### 3.2 Cancellation *(revised 2026-11, spike)*

**Decision.** Cancel is always explicit, then the statement is driven to its
end; a future is never dropped as a way to stop work.

1. **Server-side cancel.** When a session is opened the Core captures its
   backend id (`SELECT pg_backend_pid()` / `SELECT CONNECTION_ID()`). Cancel
   runs `SELECT pg_cancel_backend($pid)` (Postgres) or `KILL QUERY <id>`
   (MySQL) from a **separate** pool connection (short acquire timeout,
   falling back to a one-off connection). Cancellable pool jobs (export,
   browse) capture their own backend id at job start. A cancel is only ever
   issued while the Core still holds the target connection open, because
   MySQL reuses connection ids after close: a late `KILL` could hit a stranger.
   A failed cancel is reported, not swallowed.
2. **Drain, then decide.** After the cancel the Core keeps reading the stream
   (discarding rows) until the terminal result, bounded by a drain timeout. The
   job's `CancellationToken` stops buffering, not reading. The same path
   handles the buffer cap (2.3) and closing a tab (cancel, then close).
3. **Session policy.** An editor session is **kept** after a clean cancel:
   cancel acknowledged, stream drained, and a `SELECT 1` health check
   answered. A cancel error on the health check itself means a late cancel
   was absorbed; it is retried once. A Postgres "transaction aborted" (25P02)
   answer counts as alive: the user's manual transaction is theirs, so the
   tab shows "transaction aborted, ROLLBACK required" instead of silently
   rolling it back by reconnecting. The session is **closed** (and the next
   run opens a fresh one, with a visible "session reset" note) only when the
   drain times out, the health check fails with an I/O or protocol error, or
   a future was ever dropped mid-operation (a bug path, kept as a guard).
   Until the extended protocol is verified the same way, a cancelled
   parameterized statement (9.3) also closes the session. sqlx's `Transaction` API is never used on sessions (user transactions are
   plain SQL), so its transaction-depth cancel bugs cannot apply there.
4. **Pool policy.** Pool connections hold no user state, so a pool connection
   whose work was cancelled or ended abnormally is detached and closed, never
   returned. Short metadata queries are not cancelled; an unwanted result is
   simply ignored.
5. **Recording.** "Cancelled" is a Core fact, not inferred from the result:
   after `KILL QUERY`, MySQL may return success (`SELECT SLEEP` returns 1,
   `DO SLEEP` returns Ok) or error 1317, and Postgres 57014 also comes from
   `statement_timeout`. A job is `Cancelled` if the Core issued a cancel before
   the statement's terminal result arrived; the server outcome is attached
   (for example "server reported success: the statement may have taken
   effect"), and rows received so far are kept and marked partial.

**Why.** The spike showed dropping a future leaves the server running and
stalls the connection's next use silently (30 s for a 30 s sleep), while a
clean server-side cancel leaves the connection usable within milliseconds with
its session state intact (temp tables, settings). Discarding after every
cancel would throw that state away for no safety gain; discarding only when
the protocol state is unknown keeps both safety and the user's session. Serves
Performance, UX and No Magic.

**Trade-off.** A drain and a health-check round trip after each cancel, and
more states to test (clean, late cancel, aborted transaction, drain timeout).
The spike verified the clean path on simple-protocol queries only. If Postgres cancel proves fragile, the `tokio-postgres` escape hatch
(1.2) provides a native wire-level cancel.

#### 3.2.4 Postgres driver: implementation status and deviations *(new 2026-11)*

Recorded after the first Postgres slice and an independent review:

- **Multi-statement strings run in full on the server.** `raw_sql` executes
  every statement (`SELECT 1; DROP TABLE x` drops the table). The driver only
  reports `extra_statements_discarded` afterwards. The caller must split
  scripts first (9.2, needs the `dbased-sql` scanner).
- **The pool is lazy.** `connect` opens one direct connection, then builds the
  pool lazily, so typed connect errors surface at once. The first cancel can
  therefore pay the pool's first connect and fall back to the one-off cancel
  connection.
- **Describe runs on the user's session** (Parse of the user SQL). Inside a
  transaction it can block on locks; a timeout closes the session.
- **Abnormal close** (drain timeout, error, dropped future, failed health
  check) cancels the backend while the connection is still held, then drops
  the socket. A dropped `execute` future is only cancelled on the next call or
  on `close()`; callers must always call `close()`.
- **Unsupported for now:** COPY via `raw_sql` (closes the session), session
  idle timeout and reconnect, connect timeouts, parameterized/extended
  protocol, `list_routines`.
- **Decided (2026-11), Core-initiated cancel inside a user transaction:** a
  row cap or a gone consumer must not abort the user's `BEGIN` block. While a
  transaction is open the driver drains and discards the remaining rows
  instead of cancelling. An explicit user cancel still cancels. Requires
  reliable transaction-state tracking (sqlx hides the ReadyForQuery status).
- **Decided (2026-11), canceller API:** cancellation goes through a handle tied
  to a live session, never a bare `SessionId`, so a cancel can only reach the
  backend while the Core still holds that connection (no PID-reuse hazard).

### 3.3 Export *(revised 2026-11)*

**Decision.** CSV export streams directly from a fresh query to disk via a
`spawn_blocking` writer using the `csv` crate over a buffered file. It never
reuses the result buffer and never holds the whole file in memory. It is a Job,
emits progress on a channel, and is cancellable like any other. The writer
emits UTF-8 and offers an optional BOM for spreadsheet compatibility. JSON and
NDJSON are cheap follow-ups through `serde_json`; Parquet is a later milestone.
Export and large copy-to-clipboard are always Core jobs, never driven through
the grid widget (its `dump` helpers call the delegate for every row, see 6.3).

**Why.** Export is the one operation that legitimately touches the full result
set; it must not inherit the row cap. Streaming to disk keeps memory flat for
arbitrarily large exports. Serves Performance.

**Trade-off.** Export re-runs the query rather than reusing buffered rows. This
is correct: the buffer is capped and possibly truncated; the export must be
complete.

### 3.4 Error model

**Decision.** One `CoreError` enum built with `thiserror`, defined in `core`
and mirrored as a stable `Event::Error { kind, message, detail? }` shape in the
`commands` crate, where `kind` is a machine-readable variant name. No `unwrap`
/ `expect` outside tests. The mapping from `CoreError` to the `commands` shape
happens at the single boundary where Core executes a `Command` (0.2); no
per-call ad hoc logic anywhere else. Database errors carry the server-reported
position when available so the editor can underline the offending token.
`anyhow` is allowed only at the `app` edge.

**Why.** A stable `kind` lets the UI react to error *categories*
(auth failed vs query syntax vs connection lost) without string matching.
Serves UX, Maintainability, and the Constitution's error-handling rules.

**Trade-off.** Maintaining the enum as new error sources appear. Expected cost.

---

## Part 4 — Metadata and Explorer

### 4.1 Lazy loading

**Decision.** Connecting loads nothing but the schema list. Expanding a node
loads only its direct children, through the Driver trait's `list_*` methods.

**Why.** Directly from the Constitution; a large database has thousands of
objects and eager loading would stall the connect. Serves Performance.

**Trade-off.** More round trips as the user explores. Each is small and lazy.

### 4.2 Caching and invalidation *(revised 2026-11)*

**Decision.** The Core caches loaded metadata per connection. Two refresh paths:

- **Manual.** A "Refresh" action (per node and global) always exists, clears, and
  reloads. There is no TTL and no time-based polling.
- **Automatic, on DBased's own DDL.** When a statement executed *through DBased*
  changes the schema, the Core invalidates the affected metadata so the explorer
  reflects reality without a manual refresh. Detection does **not** use leading
  keyword matching (too brittle). It uses, in order:

  1. **Tokenize and split** the executed batch with the in-house tokenizer
     (2.7), so comments, whitespace, keyword case, strings and multiple
     statements are handled without relying on a successful parse.
  2. **AST classification** of each statement with `sqlparser` (Postgres /
     MySQL dialect), yielding statement kind and target object so invalidation
     is **scoped** (just that table or schema node), not a full-tree reload.
  3. **Server confirmation: not available today.** It would use the command tag
     the server returns, but sqlx's `QueryResult` exposes only `rows_affected`,
     so no such signal exists and the classification in step 2 is the only
     signal. Revisit only if a driver (for example a `tokio-postgres` Postgres
     driver, see 1.2) exposes the command tag.
  4. **Conservative fallback.** Anything the parser cannot confidently classify,
     or constructs that can hide DDL (`DO $$ ... $$` blocks, `CALL` procedures,
     `EXECUTE` / dynamic SQL, parse failures on exotic syntax) is treated as
     "schema may have changed" and invalidates at the connection (or current
     database) scope. We over-invalidate rather than risk staleness.

Invalidation mechanics: mark cache entries dirty; collapsed nodes re-fetch
lazily on next expand; currently visible nodes re-fetch in the background and
update in place, with a subtle in-progress indicator so the update never looks
like a spontaneous flicker.

**Why.** Stale metadata after a user's own `ALTER TABLE` is one of the most
common DBeaver complaints, and DBased should fix it. Over-invalidating is cheap
because metadata is lazy and small (4.1). This trades a strict reading of "No
Magic" for correctness, but the behavior stays predictable: it only ever reacts
to the user's own action, never spontaneously.

**Trade-off and boundary.** Automatic invalidation covers only DDL issued
*through DBased*. Schema changes from another connection or external tool are
invisible without polling, so the manual global refresh remains the escape
hatch. No client can guarantee zero staleness on a shared database without
invasive server-side hooks (event triggers), which a client must not install.

---

## Part 5 — Persistence and Security

### 5.1 Credential storage *(revised 2026-11, spike)*

**Decision.** Passwords are stored in the OS keychain through the `keyring`
ecosystem. `keyring` 4.x is split into `keyring-core` (the `Entry` API) plus one
crate per platform credential store; DBased depends on `keyring-core` and the
stores directly, selected per target OS, rather than on the umbrella crate:

- Linux: `zbus-secret-service-keyring-store` (pure Rust, no system packages)
  with exactly one runtime feature, `rt-tokio-crypto-rust` (the store has no
  default runtime; tokio is already in `core`). The libdbus-based store is
  rejected: it needs `libdbus-1-dev` to build. KWallet should work through its
  Secret Service API but is untested.
- macOS: `apple-native-keyring-store` (resolves, not yet compiled).
- Windows: `windows-native-keyring-store` (resolves, not yet compiled).

Wiring: the store is created once at Core startup and held as injected Core
state (entries are built from it, not from a process-wide default store).
Every keyring call is blocking, and an unlock prompt can block indefinitely,
so calls run on `spawn_blocking` and are never on the connect critical path:
while a prompt is pending the UI offers "enter password instead". Errors map
by variant (the enum is non-exhaustive, so there is a catch-all):
`NoEntry` means no saved password, `NoStorageAccess` (locked or dismissed)
means prompt for this connect, `PlatformFailure` (no D-Bus or no Secret
Service) means the store is unavailable for this run, with the platform
error's text kept as detail. The Core does not downcast to `secret_service` /
`zbus` errors: that would need both as direct dependencies kept in lockstep
with the store's, to change only a message.

The workspace config file stores only non-secret connection metadata (host,
port, database, user, options) plus a key referencing the keychain entry.
Plaintext passwords are never written to disk. In DBased's own code, secrets
are held in `secrecy::SecretString` and excluded from `Debug` and log output.
**Known limit, stated rather than overpromised:** `secrecy` only limits copies
on our side. `keyring-core` returns a plain `String`, D-Bus buffers hold the
secret, and sqlx's connect options keep the password in memory for the pool's
lifetime so it can open new connections.

If no Secret Service provider is present on Linux, the app falls back to
prompting per session, and may later offer an explicit, encrypted-file store as
a user opt-in (not a silent default).

**Why.** Secrets must never be on disk in plaintext. Taking the stores
directly keeps the dependency tree small and per-platform. Serves UX (no
re-typing) and security.

**Trade-off.** Slightly more wiring than a single crate. Validated on Linux
with gnome-keyring only (Part 10, item 7); macOS, Windows and KWallet are
verified when those targets are first built.

### 5.2 Configuration and workspace files *(revised 2026-11, spike)*

**Decision.** Storage is split by data shape, not forced into one store:

- **Human-facing config in a TOML file** (located via `etcetera`, XDG on Linux
  and macOS, no Tauri dependency; `directories` was dropped after 10 months
  without a release): connection metadata, settings, and the workspace (open
  tabs and their connections). Read and written with `toml` + `serde`; rewrites
  of an existing user-edited file use `toml_edit` so comments survive. Written
  with atomic replace (temp file, then rename) so a crash mid-write cannot
  corrupt it.
- **Dock layout in a separate JSON file per workspace** *(spike, 2026-11)*.
  The Dock serializes to a deeply nested tree with arbitrary JSON panel state
  (`DockAreaState`), which TOML represents badly, and nobody edits it by hand.
  The UI serializes it with `serde_json` and hands it to the Core as an opaque
  string through a `Command`; the Core writes it atomically without knowing its
  shape, so `core` stays free of GPUI types. A layout that fails to load (for
  example after a gpui-component upgrade changed the format) falls back to the
  default layout, never blocks startup, and the old file is kept as a backup.
- **Machine-managed, append-heavy data in SQLite**: query history first (later,
  saved snippets and cached metadata snapshots if they earn it). SQLite is
  reused through sqlx's sqlite backend, so it is **not a new major dependency**
  (see 1.2), and it gives durable incremental appends plus indexed search and
  pagination over a history that grows without bound. (`rusqlite` is the
  lighter alternative if sqlx is ever dropped.)
- Credentials stay in the OS keychain (5.1), never in either store.

**Why.** Config is small, mostly static, and benefits from being readable and
diffable, which a flat file gives and a database does not. History is unbounded
and needs indexed search and cheap incremental writes, which a flat file handles
badly and SQLite handles well. Serves Simplicity, Performance, and the
Constitution's human-readable-config preference.

**Trade-off.** Two stores plus a layout file instead of one store. The boundary
is unambiguous: if a human edits it, it is the TOML file; if the app appends to
it at runtime, it is SQLite; the Dock's own tree is an opaque JSON file. TOML
is awkward for deeply nested structures; the workspace schema must stay
reasonably flat. The layout format belongs to gpui-component and may break on
upgrades; losing a layout is acceptable, losing tabs or connections is not.

### 5.3 Transport security *(new 2026-11, revised after spike)*

**Decision.** TLS uses rustls with the operating system trust store (the sqlx
rustls-with-native-roots feature, and `rustls-platform-verifier` where a
non-sqlx path needs it). Connection settings expose the standard modes
(`disable`, `prefer`, `require`, `verify-ca`, `verify-full`), a custom CA file,
and client certificates. **The default for a new connection is `require`.**
sqlx's own default on Postgres is `prefer`, which silently falls back to
plaintext, so DBased always sets the mode explicitly and never inherits the
library default. Choosing `prefer` or `disable` is an explicit user choice shown
with a visible warning in the connection form and on the connection itself
(No Magic: an unencrypted connection is never an accident). *(Spike, 2026-11.)* `tls-rustls-ring-native-roots`
builds; `require` works on both databases and fails clearly against a server
without TLS. Verification against native roots or a real CA bundle is
**not yet tested** (the spike only had a self-signed certificate); this must
be verified before the verify modes ship.

SSH tunneling is a **post-MVP** feature. When built it uses `russh` (pure Rust,
tokio-native, pre-1.0 so pinned) to open a local forwarded port that sqlx
connects to (sqlx cannot accept a custom stream). Host-key verification is
mandatory with a known-hosts store; unknown host keys are never accepted
silently. TLS server-name verification against `127.0.0.1` is handled by an
explicit host override. If the tunnel needs richer stream control, the
`tokio-postgres` escape hatch (`connect_raw`) is the cleaner path for Postgres.

**Why.** Defaults must be safe, and SSH must not influence the MVP while still
having a worked-out landing place.

**Trade-off.** A local forwarded port is a small local attack surface; it binds
to loopback on an ephemeral port only.

---

## Part 6 — Frontend (GPUI)

These sections are the GPUI-era replacements for the superseded Tauri-era UI
decisions listed in 0.4. They live entirely in the `ui` crate.

### 6.1 SQL editor *(revised 2026-11, spike, human test)*

**Decision.** Use gpui-component's native code editor (`EditorState` with
`.language("sql")`) for the SQL editor; it is used as provided, not rebuilt.
Capabilities and honest limits are recorded in 0.4. No lazy-load gymnastics are
needed (it is native, not a webview), so there is no startup penalty to schedule
around. The editor is treated as a **swappable sub-component behind an
interface** (UI exchanges SQL text, selections and completion requests), so it
can be replaced or wrapped without touching the rest of `ui`.

*(Spike, 2026-11.)* SQL highlighting needs the `tree-sitter-sql` feature and
the multi-cursor actions need `gpui-base` (0.1). Large files: the editor's
own docs target about 50K lines; a 100K-line file opened in about 0.3 s and
edited at under 9 ms per keystroke, but RSS grew from 127 MB after open to
about 295-385 MB. The editor is for scripts, not dumps: files above the
upstream target (about 50K lines or 5 MB, to be tuned) open only after a
warning that names the size. Editor memory is a **watch item, not a budget**,
until it is known whether the growth is a one-time cost (highlighting, layout)
or grows with edits; the latter would be a defect to fix upstream.

*(Human test, 2026-11.)* Every editor shortcut needs keyboard focus, and the
spike's shortcuts silently did nothing until it focused the editor. Rule: the
editor takes focus when its tab is opened or activated, and a tab restores
focus to its editor when the user returns to it.

*(Human test, 2026-11.)* **Input methods and symbol entry.** gpui on X11 talks
to input methods only through XIM (the `zed-xim` crate): it needs
`XMODIFIERS=@im=<name>` and a running XIM-capable daemon (ibus, fcitx5),
otherwise it finds no server and text arrives as plain key events. There is no
IBus/fcitx D-Bus path, and `Window::show_character_palette()` does nothing on
Linux. `Ctrl+.` stays bound to the upstream code actions (`ToggleCodeActions`),
matching VS Code's Quick Fix; they become useful once an SQL language server
is attached. An emoji and symbol picker (a command-palette entry that inserts
text at the cursor, independent of any input method) is **low priority** and
may be dropped. Where an input method is connected and uses `Ctrl+.` itself,
it sees the key first and wins (to verify on XIM and Wayland). The docs list
the environment an input method needs, because a hotkey that opens nothing is
usually the desktop, not DBased.

**Why.** Native, Tree-sitter highlighted, LSP-capable, GPU-rendered. Covers a SQL
client's editing needs without a webview. Serves Performance and Simplicity.

**Trade-off.** Below Monaco in power (see 0.4). Multi-cursor basics exist
upstream; the VS Code-style commands built on them are tracked as OPEN-6. The
input component exposes no IME preedit/commit callbacks (only
`marked_text_range` and Change / PressEnter / Focus / Blur events), so IME,
dead keys and paste quality can only be checked by a manual checklist on X11
and Wayland (Part 10, item 3). The own picker is a small extra component, kept
because symbol entry would otherwise depend on each user's desktop setup.
Contained to `ui` either way.

### 6.2 UI state *(revised 2026-11, spike)*

**Decision.** UI state lives in GPUI entities (owned by the GPUI `App`), sliced by
concern: connections, open tabs, explorer/metadata view-models, jobs. Large
result-set rows do **not** live in UI entities; they stay in the Core buffer (2.3)
and the grid reads pages from there. The only rows the UI holds are the grid's
small, bounded window cache of display strings (6.3).

**Why.** Keeping bulk rows out of UI state preserves the virtualization win and
keeps UI updates cheap. Matches the Constitution's "keep state simple" and
"avoid global mutable state". Serves Performance and Maintainability.

**Trade-off.** Two homes for state (UI view-models vs Core buffers). The boundary
is clear: shared UI facts in entities, bulk data in Core.

### 6.3 Grid, tree, and docking *(revised 2026-11, spike)*

**Decision.** The data grid is gpui-component's virtualized `Table`, kept as
is, fed by windowed fetches (2.3); the explorer tree and history use its
virtualized `List` / `tree`; panel arrangement uses its **Dock** (resizable
panels, draggable tabs, nested splits, serializable layout). The grid consumes
the columns-once / rows-as-arrays payload from 2.5 directly. Grid and editor are
wrapped behind our own traits so a custom element built on `gpui-base`
primitives can replace either if a limit is hit.

*(Spike, 2026-11.)* Grid rules:

- The `TableDelegate` reports the row count and renders each cell on demand;
  `visible_rows_changed` requests the windows around the visible range (200
  rows each) from the Core, and requests for windows already scrolled past are
  dropped. The UI keeps a small LRU of windows as display-ready strings (the
  spike used 24 windows, 4,800 rows); bulk rows stay in the Core (6.2).
- `render_td` is O(1): a lookup and a clone, no formatting or decoding. Cells
  of a window not yet arrived render a deliberate loading state (a muted
  skeleton of the same height), never a layout shift.
- **Frame budget.** The grid repaints only on change (scroll, window arrival,
  selection, resize); an idle grid renders no frames. Scroll APIs such as
  `scroll_to_row` are called only when the target row changes, never per frame
  (each call notifies). Window arrivals notify once per window (cheap: about
  100 notifies/s while flinging still gave one render per frame). The grid's
  render cost must stay **under 8 ms of CPU per frame** on the reference
  machine at the default window size; the spike measured 8-11 ms (including
  its stats overlay), which leaves no headroom under CPU contention (frames
  then fall to 30 fps). Profiling and reducing that cost is part of the grid
  milestone; if the Table's own per-cell overhead cannot be brought under the
  budget, that is the trigger for the custom element above.
- Sorting is a delegate callback that sends a `Command` to the Core and reloads
  the windows; export and large copies are Core jobs (3.3), never the Table's
  `dump` / `dump_range`.

Dock rules: placements are left, right, bottom and center only (0.4). Panels
are registered with `register_panel(name, builder)` keyed by `panel_name()`,
wrapped with `panel_handle` and added with `panel_view` / `add_panel_view`
(plain `add_panel` loses tab titles). Layout round-trips through
`DockArea::dump` / `load` (sizes and per-panel state included) and persists as
the opaque per-workspace JSON file in 5.2. Unknown panel names restore as
placeholders that keep their state. The panel registry is a library-owned
global, accepted because it is confined to `ui`.

**Why.** All three are provided and GPU-virtualized, and the spike confirmed the
Table virtualizes rows rather than holding them. Removes the need to build or
wire TanStack-equivalents. Serves Performance and the Constitution's
virtualization mandate.

**Trade-off.** Tied to gpui-component's component APIs, which are pre-1.0 and
change on minor versions. Contained to `ui` and pinned per 0.1. The grid's
fixed per-frame cost is the tightest performance margin in the UI; 120/144 Hz
displays (6.9-8.3 ms per frame) are not reachable at today's cost.

---

## Part 7 — Testing *(revised 2026-11)*

**Decision.** The Core is tested with (a) unit tests using a mock `Driver` impl
(`mockall` at the trait boundary; hand-written fakes for stateful things such as
an in-memory catalog), and (b) integration tests against ephemeral Postgres and
MySQL via `testcontainers` (with `testcontainers-modules`, falling back to
`GenericImage` if a module lags the core crate), one container per test binary
and one database per test. The tokenizer and parser layers (2.7) use
table-driven tests (`rstest`) and snapshot tests (`insta`). Tooling: `cargo
nextest` for test runs (plus `cargo test --doc`), `cargo deny` for licenses,
advisories and sources, and `cargo machete` for unused dependencies. The `app`
crate stays thin enough to need almost no tests. The Core never needs a UI to be
tested; the UI has its own automated layers (7.1), kept apart from the Core's.

**Why.** Fulfills "the Rust Core must be testable without any UI". The mock
driver tests the engine logic; testcontainers tests the real driver impls; the
tokenizer is exactly the kind of component where tables and snapshots catch
regressions cheaply. Serves Testing and Maintainability.

**Standardization.**

1. **Layers, separated by cost.** Unit and tokenizer tests need no Docker and
   always run (`cargo test`). Integration tests need a container runtime and sit
   behind an `integration` cargo feature, so a plain `cargo test` stays fast for
   anyone without Docker.
2. **One source of truth for images.** Database images and versions are
   declared once, in a test-support module of `core`. Initial matrix: Postgres 17
   and MySQL 8.4; older supported versions (Postgres 14, MySQL 8.0) and MariaDB
   are added as the matrix grows. The development compose file (below) reads the
   same versions.
3. **One runner.** `cargo nextest` with `default` and `ci` profiles, plus
   `cargo test --doc` (nextest does not run doctests). The exact commands live in
   `AGENTS.md`.
4. **Automated tests never depend on a shared, long-lived database.** They use
   `testcontainers` exclusively: one container per test binary, one database
   per test.
5. **Podman is supported** through testcontainers (`DOCKER_HOST`); the project
   does not require Docker specifically.
6. **Manual development environment.** A `compose.yaml` at the repository root
   provides Postgres and MySQL with sample data for exploring the real app
   (explorer, grid, editing) by hand. It is a developer convenience and is never
   used by automated tests.

**Trade-off.** Integration tests need Docker or Podman available locally and in
CI, and are slower than tests against an already-running database because
containers must start (reusing one container per binary limits this). Keeping both
testcontainers and a development compose file means image versions live in two
places; the single declaration in item 2 prevents drift.

### 7.1 UI testing *(new 2026-11, revised 2026-11, human test)*

**Decision.** The UI is tested automatically in layers, all in `crates/ui` (and
`crates/app` where needed), none touching a real database (a fake command/event
channel stands in for the Core, 0.2):

1. **Logic, no UI.** Core and `commands` tests (above). SQL lexing, command
   dispatch and result formatting stay free of GPUI.
2. **Interaction tests, no display or GPU.** `#[gpui::test]` with
   `TestAppContext` / `VisualTestContext`: simulated keystrokes and clicks,
   actions, focus, keybindings, layout bounds (`debug_bounds`), deterministic
   time (`run_until_parked`, `advance_clock`), and state of the Table, Dock and
   editor. One shared `launch()` helper initializes `gpui_component::init`,
   fonts, theme and keybindings and wraps the view in `gpui_component::Root`.
   Structure dumps (the Dock layout dump, accessibility info per element) use
   `insta` text snapshots.
3. **Visual snapshot tests, no display server.** `gpui::HeadlessAppContext` with
   the headless renderer (wgpu/Vulkan on Linux, Metal on macOS) and
   `capture_screenshot`, which returns an image. It needs a Vulkan device (a real
   GPU or the lavapipe software driver) but no X11 or Wayland. Rules: one fixed
   window size per case, one theme per case, a **bundled font** (system fonts are
   the main source of drift between machines), and the real asset source so icons
   render. Baselines live in `tests/visual/baselines/<os>/` (Linux and macOS
   never share baselines: different text rendering), compare per channel with a
   small tolerance and a match threshold (start from Zed's 2 and 99%), write
   `*_actual.png` and `*_diff.png` on failure, and regenerate with
   `UPDATE_BASELINE=1`. Visual tests run with `--test-threads=1` because the wgpu
   device is process-wide, and fail with a clear message, never silently skip,
   when no Vulkan device exists.
4. **Optional end-to-end on a real display (local only).** Launch the app, drive
   it with `xdotool` (X11) or `ydotool` (Wayland), capture with `import`, `grim`
   or a desktop-portal screenshot, compare loosely. Fragile and compositor
   dependent, so never required in CI.

*(Human test, 2026-11.)* Shortcut and multi-cursor behavior is tested through
the real keymap, not by calling actions directly: `dispatch_keystroke` in
layer 2, XTEST events (as `xdotool` sends them, also under Xvfb) in layer 4.
Gap: the editor has no public getter for the selection count or the list of
selections, so such tests can only assert through text or a DBased-side
wrapper until upstream exposes one (OPEN-6).

Not adopted: Zed's own visual runner and `VisualTestAppContext` (macOS only), and
the `gpui-mcp` bridge (needs a patched GPUI; revisit only if it supports the
pinned snapshot).

Dev-dependencies (added when the first UI test exists, not before, to respect the
no-unused-dependencies rule): `gpui`, `gpui-component` and `gpui-pre-platform`
with their `test-support` features, `gpui-pre-wgpu` (exact pin, centralized in
`[workspace.dependencies]`), `image`, and optionally `image-compare` and `insta`.
CI on Linux installs `mesa-vulkan-drivers` and `libvulkan1` (software Vulkan);
macOS runs its own Metal baselines.

**Why.** The maintainers develop on machines with a display, so visual
regressions should be caught automatically instead of by eye. Headless rendering
gives deterministic, compositor-independent images, which the optional layer 4
cannot. The first two layers cover most behavior at low cost and without a GPU.
Serves Maintainability and UX.

**Trade-off.** The headless path does not capture window chrome or the compositor.
Baselines are per OS and need a tolerance, and the visual layer depends on a
Vulkan device and on a bundled font. In one scene, a real GPU and lavapipe gave
identical pixels; shadows, blur and gradients were not checked, so expect to
tune tolerance and to re-baseline after wgpu or driver upgrades. The a11y tree is
inactive in tests, so only per-element accessibility info can be asserted. GitHub
runner behavior (GPU and ICD availability) is unverified and must be confirmed
when CI is set up.

---

## Part 9 — Editor and session UX (DBeaver-informed)

Each item is tagged **MVP** or **Later** to respect "build only what the milestone
requires" and avoid scope creep.

### 9.1 Tab and session model (MVP) *(revised 2026-11, spike)*

**Decision.** Each editor tab is bound to exactly one connection (Constitution:
tabs belong to connections) and owns a **dedicated session** (2.2), so session
state persists across runs in that tab. A tab's editor holds multiple statements
separated by a configurable delimiter (default `;`). Multiple tabs may target the
same connection; each has its own session, opened from that connection's
settings outside its pool (2.2).
This is the DBeaver-like model: multi-tab, multi-statement per tab, one
connection per tab.

**Why.** Matches the Constitution's workspace model and the mental model the target
users already have, and makes `SET`, temp tables and manual transactions behave as
expected. Serves UX.

**Trade-off.** One physical connection per open tab (see 2.2).

### 9.2 Statement execution and result sets (run-current and run-all: MVP; parallel / separate-tab / detached window: Later) *(revised 2026-11, spike)*

**Decision.** Execution modes: run the current statement (under cursor), run the
selection, run the whole script (all statements). Statement boundaries come from
the in-house tokenizer (2.7), not blank-line guessing. One execution may yield
multiple result sets; each becomes its own result tab linked to its statement,
pinnable. Result tabs live in the bottom dock and can be dragged between dock
groups and splits like any panel. There are **no floating panels** (the Dock
has none, 0.4). Detaching a result to a separate OS window is Later: the spike
showed the same panel entity can be hosted in a second `Root` window, but its
lifecycle, focus, persistence (the dock layout does not cover extra windows)
and Wayland behavior are unvalidated. MVP ships run-current and run-all into
sequential result tabs; DBeaver's "execute each statement in a separate tab, in
parallel" is Later.

*(Revised 2026-11, spike.)* Run-all sends the split statements **one at a
time** on the tab's session, stopping at the first error (the error is tied
to its statement), rather than one multi-statement `raw_sql` call. A
multi-statement call ends at the first error too, but it reports results only
as a flat stream, gives no headers for empty results, and a cancel or row cap
on one statement would abort every statement after it. One statement per
call keeps each result, error, cap and DDL-invalidation signal (4.2) attached
to its statement, matching psql and DBeaver semantics (each statement
autocommits on its own).

**Why.** DBeaver-parity for the core editing loop, without taking on parallel
execution complexity in v1. Serves UX and Simplicity.

**Trade-off.** Multiple result sets add result-area complexity; phased in.
Run-all now depends on the splitter being right (compound bodies, `DELIMITER`,
2.7), and costs one round trip per statement, negligible interactively.

### 9.3 Query parameters (Later, near-term) *(revised 2026-11)*

**Decision.** Support `:name` named parameters and opt-in anonymous `?`,
DBeaver-style, with a bind dialog / parameters panel and remembered values.
Detection is performed by the **in-house tokenizer** (2.7), never regex: it
ignores `::` casts, string literals, comments, and Postgres dollar-quoted strings
(`$tag$ ... $tag$`), which are exactly the cases where naive detection misfires
(DBeaver has shipped bugs treating `$a` in dollar-quoted strings as a
parameter). Values are passed as **real bound parameters** (sqlx bind), never
interpolated into SQL, and the Core rewrites `:name` / `?` into the driver's
native placeholders (`$1..$n` for Postgres, `?` for MySQL) through the Driver
trait. Param values reuse the Value model (2.5). Statements with parameters run
on the extended protocol (not `raw_sql`), so their results take the binary
decoding path and its quirks (2.5).

*(Spike, 2026-11.)* Rules confirmed or added: `?` is a jsonb operator on
Postgres (`?`, `?|`, `?&`), so anonymous `?` stays opt-in and off by default
there; `:name` is a parameter only when the name directly follows a single
colon (not `::`) and the colon is not inside `[...]` (Postgres array slices
`a[1:n]`); a native `$1` typed by the user is left untouched.

**Why.** Safety (no injection), correct typing, and DBeaver-parity. Serves UX First
and Hidden Power.

**Trade-off.** The tokenizer sits in the execution path; it is small and heavily
tested.

### 9.4 Keymaps and block navigation (keymap layer: MVP-enabling; presets: Later) *(revised 2026-11, editor UX)*

**Decision.** The editor has a configurable keymap layer with selectable
presets. The **default preset is VS Code-like**; DBeaver-like and native (the
editor's upstream per-OS bindings) stay selectable, and any binding can be
remapped. Presets are needed because the same shortcut collides across tools:
VS Code Alt+Up/Down *moves lines*, DBeaver Alt+Up/Down *navigates between query
blocks*. In the default preset block navigation (which reuses the statement
boundaries from 9.2) gets a non-colliding binding, chosen with the preset, and
is always in the command palette. DBased's bindings live in its own
`bind_keys` in the editor's `"Input"` key context.

Default preset (Linux and Windows; macOS uses Cmd for Ctrl, as VS Code does):

| Action | Binding | Upstream (gpui-base 0.9.1) |
|---|---|---|
| Move line(s) up / down | Alt+Up / Down | build |
| Copy line(s) up / down | Shift+Alt+Up / Down | build |
| Add cursor above / below | Ctrl+Alt+Up / Down | exists; rebind on Linux; smart by default (9.4.1) |
| Add cursor at click | Alt+click, also Ctrl+Alt+click | Alt+click exists; Ctrl+Alt+click to verify |
| Select next occurrence | Ctrl+D | build on `add_selection` |
| Select all occurrences | Ctrl+Shift+L | build on `add_selection` |
| Skip occurrence | Ctrl+K Ctrl+D | build, needs upstream |
| Undo last cursor | Ctrl+U | build, needs upstream |
| Toggle line comment | Ctrl+/ | build |
| Format document or selection | Shift+Alt+F | 9.6 (Later) |
| Code actions (quick fix) | Ctrl+. | 6.1 |

**This mapping is VS Code's on Windows and macOS, not on Linux.** VS Code on
Linux uses Shift+Alt+Up/Down for *add cursor* and Ctrl+Shift+Alt+Up/Down for
*copy line*, and upstream gpui-base binds Shift+Alt+Up/Down to add cursor on
Linux too. DBased uses the table above on every OS (one muscle memory across
machines), so on Linux it overrides the upstream Shift+Alt binding and adds
Ctrl+Alt+Up/Down. Confirmed by the product owner (2026-11): the VS Code
Windows mapping is the default on every OS; Linux users who want VS Code's
Linux bindings remap them.

**Desktop grabs (measured on MATE / marco).** The window manager takes
Alt+click and Alt+drag (`mouse-button-modifier` = Alt), so Alt+click never
reaches the app there; Ctrl+Alt+click is the in-app alternative, and the docs
name the desktop setting. Ctrl+Alt+Left/Right switch workspaces there, while
Ctrl+Alt+Up/Down are free; other desktops (GNOME and others) commonly grab
Ctrl+Alt+Arrows, which is why upstream avoids them on Linux. A shortcut that
does nothing is most likely grabbed by the desktop: every action stays
reachable from the command palette and remappable. DBased never defaults to
Ctrl+Alt+D or Ctrl+Alt+L (common desktop "show desktop" / "lock screen").

**What upstream gives and what DBased builds.** Upstream has add cursor
above/below, Alt+click, column selection (Alt+Shift+drag, Ctrl+Alt+drag on
Linux), undo/redo, search/replace, indent/outdent, word movement, and the
public `add_selection`, `set_selected_range`, `selected_range`, `cursor`
and `selected_text`. Select all occurrences is built on `add_selection`
(5,000 selections in about 75 ms in the spike). The selection list, the undo
manager and `add_cursor_at` are private and there is no public selection-count
getter, so multi-selection-aware move and copy line with one undo step, skip
occurrence, undo last cursor and smart multi-cursor need upstream API first
(OPEN-6: upstream-first, a pinned fork only until the PR merges).

**Why.** VS Code is the editor most of the target users already know, and the
Constitution names it as the UX reference. Presets still let DBeaver users keep
their muscle memory. The keymap layer is needed anyway for the multi-cursor
commands (OPEN-6), so it is not extra scope. Serves UX.

**Trade-off.** A keymap indirection layer, required regardless by OPEN-6. The
default differs from VS Code on Linux and from the editor's upstream Linux
bindings, so it must be documented in the keymap help. Several commands wait on
upstream API.

#### 9.4.1 Smart multi-cursor (Later, on by default) *(new 2026-11)*

**Decision.** Add cursor above / below (Ctrl+Alt+Up/Down) is context-aware by
default, through DBased's own `SmartAddCursorAbove` / `SmartAddCursorBelow`
actions. A setting `smart_multicursor` (default `true`) turns it off, which
routes the binding back to the upstream actions. The behavior is inspired by
the VS Code extension Smart Multi Cursor (`idanpa/vscode-smartmulticursor`)
and reimplemented independently in DBased:

1. Take the last cursor (the first one for "above") and look at the
   character just before or just after it, allowing one adjacent space.
2. If that is a trigger, and **every** existing selection stands in the same
   relation to the same trigger, put the new cursor on the next (previous)
   line at the same Nth occurrence of that trigger plus the same offset.
3. Otherwise fall back to plain column insertion (visible columns, tab size,
   full-width characters). No next line means nothing happens.

The extension's triggers are the characters `" ' = : , ; . ( ) { } [ ] < > |`
and Tab, matched by a regex, with no notion of strings or comments. DBased
changes two things. **Triggers are SQL tokens** from the in-house scanner
(2.7): `,`, `(`, `)`, `=` and the other comparison operators, `::`, `.`, `;`,
string and quoted-identifier boundaries, and the keyword `AS`. **Context is
syntax-aware**: a comma inside a string literal or a comment is not a trigger.
The scan covers only the enclosing statement, so it stays bounded. Offsets are
counted in characters per line (the extension counts UTF-16 columns) and
converted to the editor's UTF-8 offsets at the boundary.

The scanner lives in `core`, which `ui` must not depend on (1.3), and a cursor
action cannot wait on a Command round trip. When this is built, the scanner
moves into a small pure crate (`dbased-sql`: no I/O, no async, no GPUI) used
by both `core` and `ui`, so there is still exactly one lexer (2.7). It also
needs the upstream selection getter and line helpers (OPEN-6). No code is
taken from the extension, so no licence obligation applies; crediting it as
inspiration is a courtesy. If code from it were ever copied or translated, its
MIT copyright notice must be kept, in the same commit.

**Why.** Column-shaped SQL (`INSERT ... VALUES` rows, column lists, `CASE`
branches, aligned `=` in `SET`) has lines of different lengths, where plain
column insertion lands in the wrong place. Following the structure instead
makes multi-cursor edits work on real SQL. Serves UX and Hidden Power.

**Trade-off.** A new cursor that does not always go straight down is less
literal than the upstream action (tension with No Magic, Part 8); the rule is
simple, the fallback is the plain behavior, and the setting turns it off. A
new crate, justified by the dependency rule rather than by size.

### 9.5 Step-by-step query debugging (Later; best differentiator candidate) *(revised 2026-11, editor UX)*

**Decision (future).** A debug mode for large nested and legacy queries: test
them step by step, block by block, without rewriting them by hand.

- **Block tree.** The Core parses the statement with `sqlparser` (2.7) and
  builds a tree of query blocks: CTEs, sub-queries in `FROM`, `WHERE` and the
  select list, and set-operation branches. AST spans are converted from chars
  to bytes (2.7) to map each block to its text. If the statement does not
  parse, the mode is simply unavailable for it.
- **Outline and isolated runs.** An outline panel shows the tree. Any node can
  be run alone: the Core rewrites the SQL to that node plus only the
  dependencies it needs (for example `WITH a AS (...), b AS (...) SELECT * FROM
  b`), and the generated SQL is **always shown before it runs** (No Magic).
- **Row counts per step.** Each node can show rows in and rows out, which
  finds the step where rows disappear: the differentiator. Counts run on
  request (a count over the node), as cancellable jobs (3.1, 3.2).
- **Per-node cache.** Results are kept per node, keyed by the node's
  generated SQL and parameter values, labeled with when they ran, and never
  reused after the text changes.
- **Correlated sub-queries** cannot run alone. They are marked "not runnable
  in isolation", or the user picks a row of the outer block and its values
  bind as parameters. Detection is best-effort: a qualified reference to an
  outer alias is certain, an unqualified column is ambiguous without the
  catalog, and a server "unknown column" error is reported as "probably
  correlated".
- **Safety.** A CTE can modify data (Postgres `WITH x AS (INSERT ... RETURNING
  ...)`). Any node that is not a pure read is refused before it runs (AST
  check), and fragments run inside a `READ ONLY` transaction as a second guard
  (it also stops functions that write). Fragments run on the tab's session
  (2.2) so temp tables and settings apply; if that session is inside a user
  transaction, the mode does not open its own and refuses to run.
- **Cost.** Each step re-runs its upstream steps, so results use a visible,
  configurable row limit. Optionally a node can be materialized into a temp
  table on the tab's session; that is a write, so it is opt-in, previewed and
  explicit.
- **Parameters** (9.3) carry through to every generated fragment.

Phasing: (1) CTE chains without correlation; (2) general sub-queries,
correlation by picked outer row; (3) per-step plans with 9.7; later, predicate
bisection ("why zero rows": drop `WHERE` predicates one at a time to find the
one that removes every row).

**Why.** Large companies run huge legacy SQL with many nested blocks, and
debugging them today means copying fragments by hand. Built on the parser
already in the stack. Serves UX and Hidden Power.

**Trade-off.** Real complexity: rewriting has dialect edge cases, correlation
detection is heuristic, and each step costs a query. Kept safe by refusing
non-reads, the read-only transaction and always-visible SQL; deferred until the
core loop is solid.

### 9.6 SQL formatting with portable styles (Later) *(new 2026-11)*

**Decision (future).** There is no shared standard for SQL formatting. DBased
treats formatting as a **specification problem**: a portable style format plus
a deterministic engine, so a team can define a style once and get the same
output on every machine.

- **Style file.** Declarative data only, never code, so importing someone
  else's style is safe. One TOML file with a schema version, a minimum engine
  version, a name, a description, the dialects it applies to, and its rules.
  It may extend a bundled preset and store only the differences (presets only,
  not other files, so a style never depends on a chain of remote files).
  Field names are fixed with the format; for example:

  ```toml
  format = 1
  min_engine = "1.0"
  name = "acme"
  dialects = ["postgres", "mysql"]
  extends = "preset:<name>"
  [rules]
  keyword_case = "upper"
  comma = "leading"
  max_width = 100
  ```

- **Sharing.** By file, git repository or URL. Import is explicit: the file is
  validated against its schema and stored as a local copy; a URL is fetched
  once and never updated behind the user's back.
- **Determinism.** Same input, same style file and same engine version give
  the same output on any machine. The spec ships a **conformance suite**
  (input, style, expected output snapshots) per format version; every engine
  release passes the suites of all format versions it supports. Output
  changes within a format version are bug fixes only, each adding a
  conformance case and a release note; a deliberate style change needs a new
  format version.
- **Engine.** In the Core, on top of the in-house scanner (2.7), with a light
  structural layer (parenthesis depth, clause keywords). It keeps every
  comment, never touches string, quoted identifier or dollar-quote contents,
  and leaves any region it does not understand verbatim. After formatting it
  re-scans and checks that the token stream is unchanged except for whitespace
  and keyword case; if not, it aborts and changes nothing. It does **not**
  print `sqlparser`'s AST back to text, which loses comments and layout. The
  `sqlformat` crate is only a baseline to compare against, not adopted.
- **Rules.** A small, deliberate set: keyword case, indentation, comma
  position, line breaks for `JOIN`, `AND` / `OR` and clauses, maximum width,
  plus dialect handling (dollar-quoted bodies, `DELIMITER`, MySQL `ON
  DUPLICATE KEY UPDATE`). Fewer options give more consensus (Prettier's
  lesson) and keep Simplicity.
- **No Magic.** Formatting runs only on command (Shift+Alt+F in the default
  preset, 9.4), on the whole document or the selection, as one undo step.
  Never on save or on run.
- **CI check.** A Core-only command line entry, `dbased fmt --check`, exits
  non-zero when files differ from the style, so a team can enforce it. A
  connection may override the style used for its tabs.

Phasing: (1) engine with one fixed default style; (2) style format v1 and 3-4
presets (names and contents chosen later); (3) import / export; (4) the CLI
check; (5) maybe a place to share styles. Reading sqlfluff or pgFormatter
configs is a later step, not a goal.

**Why.** Formatting debates cost teams time, and no style travels between
tools. A portable, safe, deterministic style turns DBased into the place where
a team's SQL style is defined. Serves UX and Maintainability (of the users'
SQL), and fits Core Independence (the engine runs without a UI).

**Trade-off.** Ambitious: a spec, a conformance suite and an engine to
maintain, and correctness risk (a formatter must never change meaning; the
token check guards it). Scope creep is the main danger, so the rule set stays
small and compatibility with other tools waits.

### 9.7 Explain visualization (Later) *(new 2026-11)*

This is the Constitution's "Explain Visual" future feature (Part A, Future
Features), worked out here.

**Decision (future).** Show query plans, including `EXPLAIN ANALYZE`, in a
summarized and a detailed view.

- **Sources.** Postgres: `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`, or
  `EXPLAIN (FORMAT JSON)` without executing. MySQL: `EXPLAIN FORMAT=JSON`
  for a plain plan and `EXPLAIN ANALYZE` (tree text) for actuals; whether
  MySQL 8.4 offers JSON output for `ANALYZE` is to be verified, and is
  preferred if so.
- **Common plan model.** The formats differ greatly, so the Core parses each
  into one model (like the Value model, 2.5): a tree of nodes with operation,
  relation, estimated and actual rows, loops, time (total and self), buffers
  and cost, plus the raw fields it does not model. The UI sees only this
  model; the raw plan text stays one click away.
- **Two views.** *Summary*: the slowest nodes by self time, sequential scans
  on large tables, sorts and hashes spilling to disk, large estimate-versus-actual
  divergence, nested loops with many iterations. *Detail*: the full tree with
  the bottleneck highlighted.
- **Safety.** `EXPLAIN ANALYZE` **executes** the statement. A pure read (AST
  classification, 2.7) runs as is, with a warning that it runs to completion
  and can be costly. Anything else, including anything that does not parse,
  needs explicit confirmation and runs inside a transaction that is rolled
  back. The confirmation says that a rollback cannot undo non-transactional
  effects (sequences, MySQL non-transactional tables, external side effects
  of functions). If the tab's session is inside a user transaction, a
  non-read is refused. It runs on the tab's session as a cancellable job
  (3.1, 3.2); parameters (9.3) are bound as usual.
- Ideas, not code, come from pev2, explain.dalibo.com and explain.depesz.com.

Phasing: (1) plain and analyzed plans with the slowest node highlighted; (2)
summary and detail views; (3) per-step plans inside the debug mode (9.5).

**Why.** Raw `EXPLAIN ANALYZE` output is hard to read, and the common question
is "which node is slow, and why". A model in the Core keeps both databases
behind one view and keeps the analysis testable without a UI. Serves UX and
Hidden Power.

**Trade-off.** Two plan parsers to maintain against changing server output
(unmodeled fields are kept raw, never an error). Executing a statement to
explain it is inherently risky; the rollback, the confirmation and the honest
warning bound that risk without hiding it.

---

## Part 10 — Week-One Spike *(new 2026-11, revised 2026-11, spike, human test)*

**Decision.** Before the real UI is built, a time-boxed spike validates the
riskiest assumptions of the GPUI bet and of the new SQL handling. It is a gate:
the findings are recorded here (and failures trigger the escape hatches in 0.1)
before feature work proceeds. The spike code is throwaway or lives on its own
branch; only conclusions enter this document.

Checks:

1. **Grid scale.** Scroll a table backed by 5,000,000 logical rows through
   windowed fetches from a fake Core source. Measure scroll smoothness, memory,
   and window-fetch latency. Pass: smooth scrolling and flat memory.
   **DONE (2026-11, X11, AMD GPU, gpui-component 0.9.1).** Rows are virtual;
   RSS 109 to 138 MB over 5M rows (200-row windows, 4,800-row cache). Quiet
   machine: steady 16.7 ms frames (p95 ~16.8) at 2k-500k rows/s, fetch latency
   up to 50 ms; under CPU contention 33 ms. The grid costs 8-11 ms CPU per
   frame, hence the budget rules in 6.3. *Human check (2026-11, X11/MATE):*
   scrolling to the end of the 5M rows with the mouse wheel and by dragging
   the scrollbar was smooth to the last row with correct positions: pass.
2. **Editor.** Open a 100,000-line SQL file; exercise highlighting, find,
   multi-cursor (Alt-click, add-cursor above/below, `add_selection`), soft wrap.
   **DONE (2026-11).** First render 165 ms (next frame 295 ms), insert at line
   50,000 7.7 ms avg (p95 8.3), jump to end 17 ms; RSS 127 MB after open,
   ~295-385 MB after edits (watch item, 6.1). Alt-click and `add_selection`
   (select-all-occurrences) work. Needs `tree-sitter-sql` and `gpui-base` (0.1).
   *Human test (2026-11, X11/MATE):* editor shortcuts first failed because the
   spike never focused the editor on open; reproduced under Xvfb with XTEST
   key events and fixed, after which Ctrl+D (the spike's select-all) gave
   5,000 selections (its cap), Shift+Alt+Down 2 cursors, and Ctrl+Alt+Down
   (after adding that binding) 3 cursors after two presses. Rule: the editor
   takes focus when opened and when its tab is activated (6.1). Alt-click
   never reached the app because the window manager grabs Alt+click (9.4);
   the spike's help text claiming Ctrl+Alt+Arrows on Linux was wrong.
3. **Input quality.** IME composition with CJK input, dead keys, and clipboard
   behavior; behavior on Wayland and X11 for Linux, then macOS and Windows.
   **PARTIAL (2026-11).** No preedit/commit callbacks exist, so this is a
   manual checklist only (6.1). gpui picks Wayland if `WAYLAND_DISPLAY` is set,
   else X11 if `DISPLAY` is set (`XDG_SESSION_TYPE` is ignored). *Human test
   (2026-11, X11/MATE):* Latin input including dead keys and accents, a large
   multi-line paste, and basic editing and undo work. The emoji hotkeys
   (Ctrl+. / Ctrl+;) opened nothing: no input method runs on that machine,
   and gpui on X11 talks only XIM, so this is an environment limit, not an
   app bug (6.1). Pending: CJK composition with a real input method (XIM on
   X11), and the whole checklist on Wayland.
4. **Startup.** Cold start to first interactive window. Budget: under 1 second.
   **DONE (2026-11, warm caches).** Median 136 ms (126-153, 10 runs) from
   process start to first rendered frame, release build, empty window.
   Re-measure cold (dropped caches) and once the real UI exists.
5. **Dock.** Layout save and restore round-trip; detach/float of a result panel.
   **DONE (2026-11).** `DockArea::dump` / `load` round-trips structure, sizes
   and per-panel state. No Tiles, floating or detach in 0.9.1 (left, right,
   bottom, center only); a second-window workaround exists. Result: 0.4, 5.2,
   6.3, 9.2.
6. **Tokenizer groundwork.** Behavior of `sqlparser`'s tokenizer on `:name`, `?`,
   dollar quotes and `::`; decide how much the in-house tokenizer can reuse.
   **DONE (2026-11).** sqlparser 0.64 lexes clean text correctly (`::`, dollar
   quotes, dialect comments) but fails with no partial tokens on unterminated
   input and reports char columns. Decision: the in-house scanner is the only
   lexer, sqlparser's tokenizer is a test oracle; splitting needs compound-body
   tracking and `DELIMITER` (2.7, 9.3).
7. **Keyring stores.** Compile and exercise the `keyring-core` + platform store
   wiring from 5.1, including the "no Secret Service" path on Linux.
   **DONE (2026-11, Linux only).** `keyring-core` 1.0 + zbus store round-trips
   against gnome-keyring with no system packages; calls block, so they run on
   `spawn_blocking`; no-service maps to `PlatformFailure`, locked to
   `NoStorageAccess` (5.1). KWallet, macOS and Windows untested.
8. **Driver gotchas.** Against live Postgres and MySQL (testcontainers):
   cancellation via side connection, discard-after-cancel, `raw_sql` multi-statement
   results, and the Value decoding matrix from 2.5.
   **DONE (2026-11, Postgres 17, MySQL 8.4, sqlx 0.9.0).** Side-connection cancel
   works and leaves the session usable with its state; dropped futures do not
   cancel and stall the next use; MySQL may return success after `KILL QUERY`.
   Result: keep-after-clean-cancel policy, sessions outside the pool, per-statement
   text-protocol execution (2.2, 2.3, 2.5, 3.2, 9.2). TLS verify modes untested (5.3).

**Gate status (2026-11).** Passed for starting the real UI: no finding
triggers an escape hatch (0.1), and every remaining check can only be fixed
inside `ui` (upstream fix or contained workaround). The human checks of
2026-11 (grid end-of-list, Latin input, editor shortcuts on X11) passed.
Still pending, scheduled rather than blocking: CJK composition with a real
input method (before the editor milestone closes); items 1-5 on Wayland; and
the grid on a lower-end CPU/GPU and under GPU contention (before the first
release). A failure there that cannot be fixed in `ui` reopens this gate.

**Why.** The documentation is a plan; these are the points where the plan could
be wrong in ways that are expensive to discover later. Serves Engineering
Quality Over Speed.

**Trade-off.** About a week before visible product progress. Cheaper than
discovering a grid or editor limit after the UI is built.

---

## Open decisions

### Resolved

- **OPEN-1 (Ad-hoc result cap).** Decided in 2.3: two caps, row count and memory
  budget, whichever first, user-configurable.
- **OPEN-2 (Page size).** Decided in 2.4: 200 rows, user-configurable.
- **OPEN-3 (Browse pagination).** Decided in 2.4: keyset by default (implies
  continuous scroll), `LIMIT/OFFSET` fallback for keyless tables.
- **OPEN-4 (DDL invalidation).** Decided in 4.2: automatic on DBased's own DDL,
  detected by tokenizer + AST parsing with conservative over-invalidation, not
  by keyword matching. Manual refresh covers external changes.
- **OPEN-5 (No-PK tables).** Decided in 2.6: read-only by default, explicit
  all-columns opt-in with a rollback-on-multi-row safety net.
- **OPEN-6 (SQL editor multi-cursor).** Decided and **reduced in scope
  (2026-11)**: build on the multi-cursor support that now exists upstream. Plan
  below.
- **OPEN-7 (SQL parsing strategy).** Decided (2026-11): hybrid of an in-house
  tokenizer and `sqlparser` (2.7).
- **OPEN-8 (Dependency sourcing for the UI).** Decided (2026-11): crates.io with
  exact pins, scheduled upgrades, escape hatches in 0.1.
- **OPEN-9 (UI de-risking).** Decided (2026-11): week-one spike gate (Part 10).
- **Keyring store wiring and the Linux fallback.** Decided by the spike
  (2026-11): `keyring-core` + `zbus-secret-service-keyring-store`
  (`rt-tokio-crypto-rust`), blocking calls on `spawn_blocking`, prompt-per-session
  fallback on `PlatformFailure` / `NoStorageAccess` (5.1).

### OPEN-6 detail: multi-cursor plan (revised 2026-11)

**Original plan** (before verification): build multi-cursor natively because the
editor had a single `Selection { start, end }`.

**Current state.** The editor now has a normalized multi-selection model with
Alt-click, `AddCursorAbove` / `AddCursorBelow` actions, and a public
`InputBaseState::add_selection(range, cx)` API (merged October 2026), which is
the programmatic equivalent of Alt-clicking another cursor.

**Remaining work, in order:**
1. Verify behavior (Part 10). *Spike, 2026-11:* Alt-click and
   `add_selection` work, including select-all-occurrences on a 100K-line file
   (capped at 5,000 selections in the spike). Still to check by hand: undo
   atomicity across cursors and multi-cursor painting.
2. Build **select-next-occurrence** (Ctrl+D) and **select-all-occurrences**
   (Ctrl+Shift+L) in `ui` on top of `add_selection`, as keymap-layer actions
   (9.4), then skip occurrence, undo last cursor, move and copy line, and
   toggle line comment. *(Revised 2026-11, editor UX.)* The full default
   mapping and what each command needs from upstream are in 9.4.
3. Smart multi-cursor (9.4.1), on by default.
4. Multi-cursor-aware completion (primary-cursor only until then).
5. Anything missing or buggy is fixed **upstream first**: open a design
   issue, follow the project's contribution rules, carry a pinned fork/branch
   (0.1) only until the PR merges. Known upstream asks: a public getter for
   the selections (also needed by tests, 7.1), atomic multi-selection edits
   for one undo step, removing the last added selection, and adding a cursor
   at a position without a click.

The editor stays behind the swappable sub-component interface (6.1), so the
fallback paths (a wrapped or custom editor element) remain available.

### Still open

- Emoji/symbol picker (6.1): low priority, command palette only; may be dropped.
- Formatting presets (9.6): names and contents of the 3-4 bundled presets,
  chosen by the product owner when phase 2 is scheduled.
- Documentation strategy is being defined separately and, once agreed, becomes
  its own standard (and likely a Constitution pillar).
- Part 10 remaining checks (gate status there): CJK composition with a real
  input method, Wayland runs, lower-end hardware and GPU contention.
- Detaching a result tab to its own OS window (9.2, Later): whether the
  second-`Root`-window technique is acceptable is a product call, to be made
  when the feature is scheduled.

---

## Part 8 — Constitution and philosophy alignment

This section exists so every philosophy in the Constitution is traceable to the
decisions, and so the places where a decision knowingly trades against a
philosophy are named out loud rather than hidden.

### Each philosophy, and where it is served

- **Performance First / Performance.** 1.1 (UI never blocks on I/O), 1.2
  (streaming decode), 2.2 (stateless work on the pool, sessions free after
  fetch), 2.3 (pages served from RAM, windowed grid), 2.4 (keyset + continuous
  scroll), 2.5 (compact columns-once payload), 3.2 (true server-side
  cancellation), 3.3 (streaming export), 4.1 (lazy metadata), 6.1 and 6.3
  (GPU-native editor and virtualized grid), Part 10 (measured, not assumed).
- **Simplicity First / Simplicity.** 1.2 (one DB crate, not two native drivers),
  2.3 (one bounded buffer), 2.6 (editing scoped to browse mode), MVP scope held.
  Tension named below.
- **UX First / User Experience.** 2.2 and 9.1 (session state behaves as users
  expect), 2.4 (continuous scroll), 2.6 (SQL preview and a visible read-only
  indicator), 3.1 (command returns a job id immediately), 4.2 (no stale schema
  after the user's own DDL). Editor-shortcut gap named below.
- **Hidden Power.** The all-columns edit (2.6) is power kept behind an explicit
  opt-in rather than on the surface. The command palette (a Constitution item)
  is the main vehicle here and remains a UI-design task, not yet a locked
  decision. Later: the step-by-step debug mode (9.5), smart multi-cursor
  (9.4.1) and explain visualization (9.7).
- **No Magic / explicit over implicit.** 2.5 (faithful values, `Unknown`
  fallback, never silent corruption), 2.6 (previewable SQL, explicit opt-in, no
  silent multi-row writes), 0.2 (explicit command/event boundary even in-process),
  3.2 ("cancelled" is recorded by the Core, never inferred; a session reset is
  shown, an aborted transaction is reported rather than silently rolled back),
  9.5 and 9.7 (generated SQL shown before it runs, writes only by explicit
  confirmation), 9.6 (formatting only on command, never on save or run).
  Tensions named below.
- **Maintainability.** 1.3 (workspace physically enforces independence), 2.1
  (driver trait, no giant match), 2.7 (each SQL need on the layer that fits it),
  3.4 (typed error categories), Part 7 (Core testable without UI), 0.1 and 0.2
  (containment and clean seam).
- **Engineering quality and longevity over development speed.** The whole document
  favors the durable choice over the fast one, including the Part 10 gate and the
  2026-11 re-verification. Central tension named below.
- **Low resource use.** GPU-native UI with no webview baseline, bounded buffers
  (2.3), lazy metadata (4.1), sessions released on idle (2.2). Improved versus
  the earlier webview plan.
- **Technological independence (Core is the asset, UI is replaceable).** 0.1, 0.2,
  0.5, 1.3. This is the spine that makes the GPUI bet affordable.
- **Avoid global mutable state; prefer composition and DI.** 6.2 (GPUI entities
  are owned, not ambient globals), 2.1 (traits over singletons), 1.3 (crate
  boundaries that inject rather than reach).

### Conscious tensions (named on purpose)

- **GPUI maturity vs longevity.** The biggest one. A pre-1.0 UI stack whose
  gpui-component layer depends on a snapshot (`gpui-pre`) maintained by one or two
  people at one company is in tension with "longevity". Resolved, not ignored, by
  0.1 (blast radius limited to `ui`, exact pins, scheduled upgrades, ordered
  escape hatches), 0.5 (Core untouched) and Part 10 (validated early). The
  accepted worst case is a UI rewrite, not a project rewrite.
- **Automatic DDL invalidation vs No Magic.** 4.2 trades a strict reading of "No
  Magic" for correctness. Acceptable because it reacts only to the user's own
  action and evicts a cache rather than silently mutating visible state.
- **Dedicated sessions vs Low resource use.** 2.2 holds one physical connection
  per open tab to keep session state correct. Bounded by idle release and made
  visible in the UI.
- **sqlx convenience vs sqlx cancel-safety.** 1.2 and 3.2 accept sqlx's gaps
  (no cancel token, dropped futures that stall instead of cancel, a pool that
  does not enforce hygiene) and neutralize them by policy: explicit cancel,
  drain, health check, sessions outside the pool, cancelled pool connections
  closed. The cost is a policy we must test ourselves; `tokio-postgres` stays
  the prepared escape hatch.
- **Formatting styles vs Simplicity.** 9.6 adds configuration, which Simplicity
  First warns against. Bounded by a small rule set, presets, and one portable
  file instead of a settings sprawl; the payoff is less configuration across a
  team, not more.
- **Smart multi-cursor on by default vs No Magic.** 9.4.1 places a new cursor
  by structure, not straight down. Bounded by one simple, documented rule, a
  plain fallback, and a setting that turns it off.
- **Session preservation vs simplicity of recovery.** 3.2 keeps a session
  after a clean cancel instead of always reconnecting, to preserve the user's
  temp tables and settings. Closing is reserved for unknown protocol state.
- **This document's depth vs Simplicity First.** The planning is detailed; the
  product is not. This is write-once reference material, not runtime complexity,
  so it does not violate the pillar. Worth rereading rarely, not constantly.
- **Grid render cost vs Performance.** 6.3 keeps gpui-component's Table even
  though it uses 8-11 ms of CPU per frame and drops to 30 fps under CPU
  contention. Bounded by an explicit budget and a named trigger for a custom
  grid element, instead of building one up front.
- **Editor below Monaco vs UX First.** 0.4 and 6.1 accept a weaker editor (rich
  find-and-replace, minimap, folding, snippets, VS Code keymap absent) as the
  price of the GPUI bet. Contained to `ui`. The VS Code-like keymap is rebuilt
  in-house (9.4), partly waiting on upstream API.
- **Dependency count vs "fewer dependencies".** GPUI, gpui-component, sqlx,
  sqlparser, the keyring stores, etcetera and the later russh each carry weight.
  Each is justified at its decision; the GPUI stack specifically is the cost of
  the UI bet, accepted knowingly.
