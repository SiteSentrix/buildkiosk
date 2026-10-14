# Assets

Every image, icon, font or other non-code asset in this repository has a row here: path, author, source,
licence and notes. `cargo xtask assets` (part of `cargo xtask ci`) fails on any asset file without one. Rules:
[`AGENTS.md`](AGENTS.md) › Assets.

BuildKiosk ships **no asset files in the program**; the only images are screenshots in the README. The interface icons are SVG paths written in Rust
(`crates/web/src/icons.rs`, original work, MIT OR Apache-2.0) and the stylesheet is code
(`crates/web/src/style.rs`). Fonts are the reader's system fonts.

| Path | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `docs/images/screenshot-project-home.png` | BuildKiosk contributors | Screenshot of BuildKiosk showing its fictional demo company | MIT OR Apache-2.0 | README only |
| `docs/images/screenshot-schedule.png` | BuildKiosk contributors | Screenshot of BuildKiosk showing its fictional demo company | MIT OR Apache-2.0 | README only |
| `docs/images/screenshot-budget.png` | BuildKiosk contributors | Screenshot of BuildKiosk showing its fictional demo company | MIT OR Apache-2.0 | README only |
| `docs/images/screenshot-rfi.png` | BuildKiosk contributors | Screenshot of BuildKiosk showing its fictional demo company | MIT OR Apache-2.0 | README only |
| `docs/images/screenshot-rfi-es.png` | BuildKiosk contributors | Screenshot of BuildKiosk (Spanish interface) showing its fictional demo company | MIT OR Apache-2.0 | README only |
