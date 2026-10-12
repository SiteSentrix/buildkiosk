# BuildKiosk — instructions for agents

BuildKiosk is a clean-room, open-source, Rust-native construction management platform targeting Procore parity
(and beyond): projects, directory and permissions, RFIs, submittals, daily logs, punch lists, inspections,
documents and drawings, scheduling, and the financial tools (budget, change management, commitments, invoicing).
It runs as one self-hosted server binary with a server-rendered web UI that needs no JavaScript, and agents can
drive all of it over the HTTP API, the CLI and MCP.

It replaces the TypeScript BuildKiosk (`bwattendorf/ConstructionManager`, React + Express + PostgreSQL), which is the
owner's own code: port its behaviour freely, but never port anything in it that was copied from Procore (wording,
templates, layouts) — re-derive that from public documentation instead (see Clean-room).

**Scope (owner decision, 2026-10-12): Procore's tools only, for now.** Build the tools Procore offers, in the order of
[`docs/gaps.md`](docs/gaps.md), with our own look and wording. Every BuildKiosk-only feature is parked (badge
printing and scan kiosk, field clock, equipment rentals, receipts, AI assistants, blockchain, AR/VR, drones, IoT,
digital twin, carbon/ESG, third-party integrations…): don't build or plan them until the owner un-parks them. The
predecessor app stays a reference only for how it did the tools both products share. Keep it simple: one tool at a
time, finished end to end (engine, tests, pages) before the next.

## Start every session here
1. Read [`ROADMAP.md`](ROADMAP.md) (stage, numbers by dimension, what's next) and the ranked work list in
   [`docs/gaps.md`](docs/gaps.md). Unless the user gives you a task, pick work from that list.
2. Read `plan/STATUS.md` if it exists (local session notes, may lag the ROADMAP) and the relevant section of
   [`docs/architecture.md`](docs/architecture.md). Behaviour reference: public documentation only (see Clean-room);
   `plan/reference/*.md` holds local notes from it.
3. Work in this order: orient → plan → implement + test → verify (run the app and look) → record (progress docs) →
   commit. Don't stop to ask unless the decision is the owner's: licensing, branding, pricing, deleting data,
   publishing, anything that touches production.

`plan/` is gitignored (local only).

## Non-negotiables
- **Clean-room: never use Procore software (absolute rule, from the project owner).** Procore's terms forbid
  reverse engineering and using the service to build a competing product. So:
  - Never sign up for, log in to, trial, script, scrape, screenshot or measure Procore (web, mobile or desktop apps,
    sandboxes, the developer portal, its API), and never compare our output against it. Don't make test data with
    it, not even "synthetic" data.
  - Never copy Procore-authored code, text or data into this repo, tests included: no help-article text, email or
    notification wording, default lists (permission templates, inspection checklists, observation types, statuses
    beyond plain industry terms), report layouts, SDK or client-library code, export files or sample projects.
  - Behaviour comes only from public documentation (Procore's public support site, release notes, public
    webinars and forum posts), cited by URL and written in our own words, and from data users send us that they
    own (used locally, never committed). Never copy Procore icons, colours, illustrations, layouts or wording
    beyond tool names that are plain industry terms (RFIs, submittals, punch list, daily log).
  - Third-party standards: never ship AIA contract documents or their form layouts and text (our payment
    applications are designed from first principles), CSI MasterFormat/UniFormat number-and-title lists, or other
    copyrighted classification lists. Users import their own cost code lists; the starter list in
    `crates/engine/src/cmd/company.rs` is original.
  - If a task seems to need any of this, stop and say so instead. `cargo xtask cleanroom` (run by
    `cargo xtask ci`) fails on vendor copyright notices and on fixtures exported from other products.
  - Never copy GPL/AGPL/LGPL code, and never add a dependency under those licences (`cargo xtask licenses`).
- **Assets: no third-party iconography or images — ever.**
  - Every image, icon, font or other asset must be original work created for BuildKiosk by a contributor (licensed
    MIT OR Apache-2.0), OSI open source, public domain / CC0, or Creative Commons with redistribution allowed.
  - **Every asset file must have a row in [`ASSETS.md`](ASSETS.md)** (path, author, source URL, licence, notes);
    `cargo xtask assets` (run by `cargo xtask ci`) fails on any unattributed asset.
  - Prefer art generated in code (the icons in `crates/web/src/icons.rs` are SVG paths written in Rust). If the
    provenance of an asset is unclear, don't add it. The BuildKiosk logo and name are the owner's brand: don't add
    the logo until the owner decides its licence.
  - Code and original assets are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`).
- **Never crash: no panics in shipped code.** A crash drops every request in flight and can lose a change a user
  just made. Form posts, API and MCP messages, imported files and command parameters are all untrusted input.
  - Fallible work returns `Result` (or `Option`); callers handle it or propagate it with `?`. Failures reach the
    user as an error message (`EngineError` for commands, the crate's own error type below the engine), never as an
    abort.
  - Workspace clippy lints ban these outside tests, so `cargo xtask ci` fails on them: `unwrap()`, `expect()`,
    `panic!`, `unreachable!`, `todo!`, `unimplemented!`, and indexing/slicing (`v[i]`, `&s[a..b]`). Use instead:
    `?`, `.ok_or(…)?`, `let … else { return … }`, `if let`, `unwrap_or` / `unwrap_or_default` / `unwrap_or_else`,
    `.get(…)`, `.first()` / `.last()` / `.split_first()`, and a real error for a match arm you believe is impossible.
  - Don't panic implicitly either: don't divide integers by something that can be zero, cap sizes and counts read
    from input (allocations, uploads, list lengths, loops), use checked arithmetic for money (`Money` and `Percent`
    only offer checked operations), and check that floats are finite before casting them.
  - Don't silence errors just to satisfy the lint: use `let _ =` or `.ok()` only where ignoring the failure is the
    right behaviour, with a comment saying why.
  - Tests may unwrap, panic and index (`#[test]`, `#[cfg(test)]`, `tests/`, `testkit`). Integration-test files start
    with an `#![allow(…)]` for that.
  - `buildkiosk_engine::guard` is the last line of defence: `Engine::execute` runs every command on a copy of the
    company state, catches a panic, drops the copy and reports `EngineError::Internal`; the HTTP server and the MCP
    server catch panics per request. It exists for bugs and doesn't replace `Result`.
  - Untrusted input is fuzzed: `crates/engine/tests/command_sweep.rs` (every command with junk parameters, as every
    kind of actor) and `crates/engine/tests/journal.rs` (random command sequences replay to the same state). Extend
    them when you add a command family, an importer, a parser or a protocol method.
- **Everything is a command.** User-visible behaviour = a `CommandSpec` in `crates/engine/src/cmd/*` (id `tool.verb`,
  label, tool, access level, params doc, run) + tests. The web UI, the HTTP API (`POST /api/v1/commands/{id}`), the
  CLI (`buildkiosk run`) and MCP all reach the same registry, so anything a person can do an agent can do. Queries
  are commands too (`rfi.list`), and aren't journaled.
- **Permissions live in the engine.** Every command declares its tool and the level it needs (`Access::Project`,
  `Access::Company`, `Access::Member`, `Access::System`); the engine checks it before the command runs, and the
  command checks item-level rules itself (authors may edit their own drafts; private items are hidden from people
  who aren't part of them). The UI hides what a person can't do, but never relies on hiding: every rule is enforced
  in the engine. A record id from another project must answer "not found" (`ctx::in_project`).
- **The journal is the source of truth.** Every change is a `JournalEntry` (command, actor, time, parameters and the
  rows it changed), appended and fsynced before the change takes effect. It is append-only: never rewrite or
  truncate it (except an incomplete last line after a crash, which the store drops with a warning).
  - Commands must be deterministic given the state, the actor, the time and the parameters: no clocks, randomness or
    I/O inside a command (the caller passes the time).
  - Schema changes are additive: new fields get `#[serde(default)]`; never rename a table, a field, or a `choice!`
    key once released. If a change would make older builds misread new data, bump `SCHEMA` in
    `crates/model/src/state.rs`; older builds then refuse to open it.
  - Secrets never enter the journal or the company state: password hashes, sessions and API tokens live in the
    server's credential store (`crates/store`).
- **Tenant isolation.** Each company is a separate state, journal and blob store; nothing reads across companies.
- **Layering** is enforced by `cargo xtask layers` (table in `xtask/src/layers.rs`). Nothing below L6 depends on the
  web stack (axum, tokio, hyper, tower, maud) or touches the file system or network.
- **The UI is thin**: pages read engine state (through `buildkiosk_engine::view` helpers, so pages and queries apply
  the same visibility rules) and act only through commands. Colours and spacing come from the theme tokens in
  `crates/web/src/theme.rs`.
- **Rust only, and no JavaScript at all** (no handwritten JS/TS, no vendored JS libraries). Pages are server-rendered
  HTML with plain forms and are served with `Content-Security-Policy: script-src 'none'`. Interactive pieces that
  truly need client code (drawing markup, Gantt dragging, offline field mode) will be Rust compiled to WASM on top
  of the same engine: **never break wasm** (`cargo xtask wasm` builds L0–L5 for `wasm32-unknown-unknown`).
- **Security basics:** argon2id password hashes, HttpOnly SameSite session cookies, a CSRF token on every form,
  rate-limited sign-in, size limits on every body and upload, uploaded files served as attachments with
  `nosniff`. Never log passwords, tokens or session cookies.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D warnings including the no-panic lints,
  tests, assets, brands, cleanroom, licenses, layers, wasm). One task id per commit (`T4.2: submittal workflow`).

## Running and looking at the app
- `cargo run -p buildkiosk -- demo --data ./demo-data` creates a demo company with sample projects (all fictional:
  never use real companies' names), then `cargo run -p buildkiosk -- serve --data ./demo-data --bind 127.0.0.1:8080`
  and sign in at `http://127.0.0.1:8080` (the demo prints its sign-in).
- `buildkiosk run --data DIR --company SLUG --as EMAIL|system COMMAND '{json params}'` runs any command;
  `buildkiosk commands` lists them; `buildkiosk mcp --data DIR --company SLUG --as EMAIL` is the MCP server
  (see [`docs/api.md`](docs/api.md)).
- **For UI work, look at the result**: take a screenshot with the preinstalled headless Chromium
  (`chromium --headless --screenshot=/tmp/shot.png --window-size=1280,900 URL`; sign in first or use a page that
  doesn't need it) and read the PNG, at desktop and phone widths. Page tests in `crates/web` render pages to HTML and
  assert on them without a browser.
- Shell gotcha in some environments: `mv`/`cp` are aliased interactive — use `/bin/mv -f` / `/bin/cp -f`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent (or a git worktree each); edit only the files you own;
  register new modules and tables under your area's `// --` marker so merges stay clean. Delete your target dir when
  you finish.

## Roadmap and progress docs
- [`ROADMAP.md`](ROADMAP.md): stage, headline numbers, dimensions, tools, upcoming, progress log.
- [`docs/target-app-parity.md`](docs/target-app-parity.md): the authoritative parity assessment (tool-by-tool table,
  weights, methodology, evidence of what ships).
- [`docs/gaps.md`](docs/gaps.md): every known shortfall, ranked; the work list. [`docs/roadmap.md`](docs/roadmap.md):
  milestones and current focus.
- [`docs/architecture.md`](docs/architecture.md), [`docs/development.md`](docs/development.md),
  [`docs/api.md`](docs/api.md), [`docs/deployment.md`](docs/deployment.md),
  [`docs/data-migration.md`](docs/data-migration.md), [`docs/ui-parity.md`](docs/ui-parity.md).

When a task lands, update in the same commit: the tool's row in `docs/target-app-parity.md` (score, missing items,
hours) and its evidence, the gap entry in `docs/gaps.md` if it closed or shrank, the milestone row in
`docs/roadmap.md`, and the ROADMAP's tables and progress log when a headline number moves. Update each touched doc's
status line.
- Grade by behaviour against the public documentation, not by whether a page exists. Scores are self-assessed, so
  err low; mark each number measured or estimated.
- Keep the README's Status section in step with the ROADMAP headline.
