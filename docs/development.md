# Development

_Status: 2026-10-13._

Read [`AGENTS.md`](../AGENTS.md) first: it holds the rules (clean-room, no panics in shipped code, no JavaScript,
everything is a command, additive schema changes).

## Everyday commands

- `cargo xtask ci`: the gate before every commit: fmt, clippy (`-D warnings`, including the no-panic lints),
  tests, assets, brands, cleanroom, licenses, layers and the wasm build. It takes a few minutes; run it in the
  background and read the log.
- `cargo run -p buildkiosk -- demo --data ./demo-data` then `serve --data ./demo-data --bind 127.0.0.1:8080`
  to look at the app. Take screenshots with headless Chromium when changing pages, at desktop and phone widths.
- `buildkiosk commands` lists every command and its parameters; `buildkiosk run --data DIR --company SLUG --as
  EMAIL COMMAND '{json}'` runs one.

## Adding a tool

1. Model: a struct in `crates/model/src/`, registered with `record!` and added to the `tables!` list in
   `state.rs` (new fields on old records get `#[serde(default)]`).
2. Engine: a module in `crates/engine/src/cmd/` with its `COMMANDS`, registered in `cmd/mod.rs`. Check item-level
   permissions in the command, validate every input, return errors instead of panicking. Tests in
   `crates/engine/tests/` use `buildkiosk_testkit::World` (`ok`, `err`, `member`, `assert_replay`); the junk-input
   sweep in `command_sweep.rs` covers new commands automatically.
3. Pages in `crates/web/src/`, a `Nav` entry in `layout.rs`, routes in `crates/server/src/lib.rs`, and a server
   test that drives the browser flow with `tower::ServiceExt::oneshot`.
4. Update `ROADMAP.md` and `docs/gaps.md` in the same commit.

## Gotchas

- Forms post to `/do/{command}`. Fields named `x[]` become lists, `rows.N.field` become lists of objects and
  `answers.key` an object.
- Inline `style=` attributes are blocked by the CSP: use classes, or SVG attributes for data-driven drawing.
- `mv` and `cp` may be aliased to interactive mode in some shells: use `/bin/mv -f` and `/bin/cp -f`.
