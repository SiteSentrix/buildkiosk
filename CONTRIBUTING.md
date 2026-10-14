# Contributing to BuildKiosk

Thanks for helping. BuildKiosk is a clean-room, Rust-only construction management platform; a few rules are strict
because they protect the project and its users. [`AGENTS.md`](AGENTS.md) holds the full list (it is written for
agents but applies to everyone). The short version:

## Rules that decide whether a change can be merged

- **Clean-room.** Never use, sign in to, scrape, screenshot or compare against Procore or any other vendor's
  software, and never copy vendor text, templates, layouts, icons, default lists or sample data. Behaviour comes
  from public documentation, written in your own words with the URL cited. If a task seems to need otherwise, stop
  and say so.
- **Licences.** Code and original assets are MIT OR Apache-2.0. No GPL, AGPL or LGPL code or dependencies
  (`cargo xtask licenses`). By contributing you agree your work is offered under MIT OR Apache-2.0.
- **Assets.** No third-party icons or images. Every asset file needs a row in [`ASSETS.md`](ASSETS.md).
- **No panics in shipped code.** No `unwrap()`, `expect()`, `panic!`, `todo!` or indexing outside tests; return
  `Result` and show the error to the user.
- **No JavaScript.** Pages are server-rendered HTML. Client code is Rust compiled to WebAssembly, and only where the
  owner has approved it.
- **Everything is a command**, with a permission level checked in the engine and tests.
- **Additive schema changes only.** Never rename a table, field or choice key once released.

## Workflow

1. Pick work from [`docs/gaps.md`](docs/gaps.md), or open an issue first for anything else. Scope is Procore's tools
   only for now; BuildKiosk-only features are parked.
2. Read [`docs/development.md`](docs/development.md) (adding a tool, running the demo, taking screenshots).
3. Make one tool or one fix at a time, finished end to end: engine, tests, pages.
4. Run `cargo xtask ci` (fmt, clippy, tests, assets, brands, cleanroom, licenses, layers, wasm). It must pass.
5. Update the progress docs the change touches (`docs/target-app-parity.md`, `docs/gaps.md`, `ROADMAP.md`).
6. Open a pull request and fill in the template. One task id per commit, for example `T4.2: submittal workflow`.

## Security problems

Report them privately, as described in [`SECURITY.md`](SECURITY.md), not in a public issue.
