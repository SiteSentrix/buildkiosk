# Development

_Status: 2026-10-13._

Read [`AGENTS.md`](../AGENTS.md) first: it holds the rules (clean-room, no panics in shipped code, no JavaScript,
everything is a command, additive schema changes).

## Everyday commands

- `cargo xtask ci`: the gate before every commit: fmt, clippy (`-D warnings`, including the no-panic lints),
  tests, assets, brands, cleanroom, licenses, layers and the wasm build. It takes a few minutes; run it in the
  background and read the log.
- `cargo xtask viewer`: builds the browser sheet viewer (WebAssembly) into `crates/server/assets/viewer`, which the
  server embeds when it is compiled (rebuild the server after it). Needs `wasm-bindgen-cli` at the version in
  `Cargo.lock` (the command tells you). Optional for development: without it the server-drawn viewer is used.
  `cargo xtask ci` only type-checks the viewer for wasm; the CI workflow builds it in a separate job.
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

## Languages

The pages are written in English and shown in English, Spanish, French or Dutch (the links at the foot of every page;
the choice is a cookie, kept with the account too when signed in (`person.set_language`; the saved choice wins and follows the person to other browsers), and the browser's own preference is used until one is made). PDFs are translated the same way when they are rendered (`pdf::with_language`, set by the server around each document). A page is translated on its way out
(`crates/web/src/i18n.rs`): each piece of wording is looked up in the language's catalog, and anything the catalog
doesn't know (names, titles and notes people typed, numbers) is left as it is. Dates are written the local way. Nothing
stored changes.

- `crates/web/i18n/en.txt` lists every piece of wording the catalogs must cover; `es.txt`, `fr.txt` and `nl.txt` have one
  `English<TAB>translation` line for each. `{}` stands for a live value (a name, a number, a date) and must be in the
  translation too (`{1}`, `{2}` when the order changes). A piece of wording ends at ` · ` or `: `, so a line like
  `Plan #4: Closeout · 3 of 7 items done` is looked up in three pieces.
- **When you add or change wording on a page**, run `cargo xtask i18n --update` (it builds the demo company in a
  temporary folder, visits its pages and compares them with the text in the source; `buildkiosk i18n-harvest` does the
  same), then add the new lines to the three catalogs. `cargo xtask ci` fails (step `i18n`) when a page says something
  `en.txt` doesn't have, and a test fails when a catalog lacks a line of `en.txt`, has a value (`{}`) missing, or has
  a line the pages no longer say.
- Not translated: text built from several live values with nothing of its own around them, messages from the engine
  (errors, notices after a save), PDFs, emails, the spoken hints of the browser sheet viewer, the API and MCP.
- To add a language: a `Lang` variant in `i18n.rs`, its months, days and range word, a catalog file with every line of
  `en.txt`, and a name for the links at the foot of the page.

## Gotchas

- Forms post to `/do/{command}`. Fields named `x[]` become lists, `rows.N.field` become lists of objects and
  `answers.key` an object.
- Inline `style=` attributes are blocked by the CSP: use classes, or SVG attributes for data-driven drawing.
- `mv` and `cp` may be aliased to interactive mode in some shells: use `/bin/mv -f` and `/bin/cp -f`.
