# Release-readiness audit (2026-10-31)

_Status: phase 1 of issue #33 (audit only; nothing fixed yet). Read from the repository; no live URL was checked._

| # | Check | Result | Notes |
| --- | --- | --- | --- |
| 1 | Landing page | FAIL | None in this repo (no site folder, no Pages workflow, no URL). README is the only front page: has hero text, 4 screenshots, tool tables, downloads table. No pricing/free messaging beyond the licence badge, no beta signup, no sibling cross-links. |
| 2 | Installers | PARTIAL | `release.yml` on a `v*` tag builds Linux (static), macOS and Windows archives (not .exe/.msi/.dmg/AppImage/.deb installers), a multi-arch container image, SHA256SUMS, attestations, `install.sh`. Windows signing (Azure) and macOS sign + notarize are wired and optional: need the owner's secrets. |
| 3 | Downloads | PARTIAL | README downloads table and `install.sh` exist. No release published yet, so no `/releases/latest` asset exists. |
| 4 | Web app | N/A | Self-hosted product; `docs/hosting.md` is a parked design. No hosted demo, so no host or health check. (A `/healthz`-style route was not found in `crates/server/src/lib.rs`; confirm.) |
| 5 | Basics | PARTIAL | PASS: README, LICENSE-MIT + LICENSE-APACHE, version 0.1.0, screenshots, in-app favicon (`/static/favicon.svg`), SECURITY.md. FAIL: CHANGELOG (the ROADMAP progress log is the only history), OG image, privacy policy, analytics (none; fine for self-hosted, nothing to phone home). |
| 6 | Signup | FAIL | No beta/waitlist link. |
| 7 | Blockers | see below | |

## Blockers
1. No release tagged: `v0.1.0` not pushed, so nothing to download.
2. README says "a clean-room Procore clone" (line 4 and the "What it is" section). Public "clone" language conflicts with the project's own clean-room posture; suggest "construction management software, written in Rust" and keep the trademark note at the foot.
3. No landing page, OG image, CHANGELOG or beta link.
4. Release workflow has not run on a real tag; signing secrets unconfirmed.

## Closeness to launch
Closest of the family on engineering (release pipeline and signing already built); furthest on marketing surface.

## Next 3 actions for the owner
1. Approve phase 2 for this repo (landing page from the shared template, CHANGELOG, OG image, beta link).
2. Decide the README wording ("clone" out?) and supply signing secrets in the `release` environment.
3. Tag `v0.1.0` once the above lands.
