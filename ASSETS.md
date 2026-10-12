# Assets

Every image, icon, font or other non-code asset in this repository has a row here: path, author, source,
licence and notes. `cargo xtask assets` (part of `cargo xtask ci`) fails on any asset file without one. Rules:
[`AGENTS.md`](AGENTS.md) › Assets.

BuildKiosk currently ships **no asset files**: the interface icons are SVG paths written in Rust
(`crates/web/src/icons.rs`, original work, MIT OR Apache-2.0) and the stylesheet is code
(`crates/web/src/style.rs`). Fonts are the reader's system fonts.

| Path | Author | Source | Licence | Notes |
|---|---|---|---|---|
