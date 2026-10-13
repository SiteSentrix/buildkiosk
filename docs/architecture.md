# Architecture

_Status: 2026-10-13._

BuildKiosk is a Cargo workspace in layers. A crate may depend only on layers below it (`cargo xtask layers`
enforces the table in `xtask/src/layers.rs`).

| Layer | Crates | Role |
| --- | --- | --- |
| L0 | `core` | ids, money (checked arithmetic), dates, text rules |
| L1 | `model` | the company state: one table per record kind, row-level diffs |
| L2–L5 | `engine`, `testkit` | commands, permissions, queries, the journal format |
| L6 | `store`, `web`, `server`, `mcp`, `app` | files on disk, HTML pages, HTTP, MCP, the `buildkiosk` binary |

Layers up to L5 build for `wasm32-unknown-unknown` (`cargo xtask wasm`); nothing below L6 touches the file
system, the network or the clock.

## Everything is a command

User-visible behaviour is a `CommandSpec` in `crates/engine/src/cmd/*`: an id (`rfi.create`), a label, a tool,
the access level it needs, a parameter description and a function. Queries are commands too and are not
journaled. The web UI (`POST /do/{command}`), the HTTP API (`POST /api/v1/commands/{id}`), the CLI
(`buildkiosk run`) and MCP (`buildkiosk mcp`) all reach the same registry, so anything a person can do an
agent can do. Shared workflows are written once: work items (punch list, observations, tasks, incidents) in
`workitem.rs`, cost documents in `costdoc.rs`, invoices in `invoicedoc.rs`.

## Permissions live in the engine

`access::check` runs before every command: the tool and level it declares against the actor's template and
per-tool overrides on the project. The command then checks item-level rules (authors edit their own drafts,
private items are hidden from people who are not part of them). Ids from another project answer "not found".
The UI hides what a person can't do, but never relies on hiding.

## The journal is the source of truth

`Engine::execute` runs a command on a copy of the state, diffs it into row changes, appends a `JournalEntry`
(command, actor, time, parameters, changes) and fsyncs it before the change takes effect. A snapshot is written
every few hundred entries; loading replays the journal over the latest snapshot. Commands are deterministic
given the state, actor, time and parameters. Schema changes are additive; the schema number in
`crates/model/src/state.rs` makes older builds refuse data they would misread. `buildkiosk check` replays every
company, `buildkiosk backup` copies and verifies a data directory.

## Pages and the server

Pages are server-rendered HTML (maud) with plain forms and no JavaScript. Responses carry
`Content-Security-Policy: script-src 'none'` and inline `style` attributes are blocked, so charts such as the
Gantt are inline SVG. Sessions are signed cookies, forms carry a CSRF token bound to the session, sign-in is
rate-limited, bodies and uploads are size-limited, and uploads are served as attachments (the one exception is
photos, which are checked by their first bytes and shown inline as JPEG, PNG, GIF or WebP only).

## Tenants

Each company is a separate state, journal and blob store under `companies/<slug>`; nothing reads across
companies. Secrets (password hashes, API tokens, sessions) live in the credential store, never in the journal.

## The sheet viewer

Drawings have two viewers on one page; the second improves the first and is optional.

**Server-drawn (always works, no scripts).** The server (L6) draws a PDF page into a PNG with a pure-Rust renderer
(`crates/server/src/render.rs`, using `hayro`), keeps the newest pictures in memory (64 MB), and the viewer page
(`crates/web/src/drawings.rs`) shows the picture with the markups drawn over it as inline SVG. A markup tool is a
plain form around `<input type="image">`: the browser sends the click position, the server turns it into a
fraction of the page, and saving goes through the `drawing.markup_add` command like everything else. Comparing two
revisions lays both pages over each other as one more picture. The viewer treats the file as untrusted (size caps,
no panics out of the renderer: a failure is a "can't be shown" page).

**Browser (WebAssembly).** `crates/viewer` (L6, built for `wasm32-unknown-unknown` by `cargo xtask viewer`) is the
same renderer compiled for the browser. The page (when the build carries the module and the file is a PDF) adds a
canvas, a toolbar and a hidden markup form beside the server-drawn parts; the module fetches the sheet file from the
existing download route, draws the page into an off-screen canvas, and shows it with smooth pan, wheel and pinch
zoom (it draws the page again, sharper, when the view settles, within 4096 pixels a side and 16 million in all).
Markups come from the page as JSON and are drawn over the picture. Placing one fills in the page's own form
(`drawing.markup_add`, with the CSRF token) so saving, permissions and the journal are exactly the same. The
module sets a class on the page only after the first page drew, so any failure (no WebAssembly, an unreadable
file, a file over 150 MB) leaves the server-drawn viewer in place. The pure parts (view maths, markup shapes, the
PDF drawing) are tested on the host; the browser glue is checked in a real browser. A panic inside the renderer
can't be caught in WebAssembly, so it stops the module (the page stays as it was); the server renders in a
blocking task that catches it.

Build: `cargo xtask viewer` (needs `cargo install wasm-bindgen-cli --version <the version in Cargo.lock> --locked`)
writes `crates/server/assets/viewer/{start.js,viewer.js,viewer_bg.wasm}`; then compile the server. `build.rs` embeds
the three files when all exist. The module is about 5 MB (not compressed on the wire yet). Its policy is set on the
viewer page only, never globally.
