## What and why

<!-- What does this change, and why? Link the issue or the gap (docs/gaps.md) it works on. -->

## How it was tested

<!-- Tests added, and how you checked the behaviour (web UI screenshots, HTTP API, CLI, MCP). -->

## Checklist

- [ ] `cargo xtask ci` passes (fmt, clippy, tests, assets, brands, cleanroom, licenses, layers, wasm).
- [ ] No panics in shipped code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`
      or indexing outside tests; errors are returned as `Result` and shown to the user.
- [ ] New behaviour is a command with a permission level and tests; item-level rules are enforced in the engine.
- [ ] Clean-room: nothing copied from Procore or other vendors (text, templates, layouts, icons, data) or from
      GPL/AGPL code ([AGENTS.md](../AGENTS.md)).
- [ ] Every new asset has a row in `ASSETS.md`.
- [ ] Progress docs updated (docs/gaps.md, ROADMAP.md) if a gap closed or shrank or something new ships.
