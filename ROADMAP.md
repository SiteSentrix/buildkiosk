# BuildKiosk roadmap

_Status: 2026-10-20. Stage: every scoped tool is built end to end and most go well past a first slice (see the progress log). Numbers are estimated, not measured._

Scope (owner decision, 2026-10-12): Procore's tools only; BuildKiosk-only extras are parked (see AGENTS.md).
The ranked work list is [`docs/gaps.md`](docs/gaps.md).

## Where we are
| Area | State |
| --- | --- |
| Engine (commands, permissions, journal) | company setup, directory, permission templates, projects, project team, cost codes, RFIs, submittals, specification sections, notifications, daily logs, punch list, observations, inspections, budget, commitments, direct costs, change events, commitment change orders, budget changes, prime contract, prime change orders, owner invoices, commitment invoices, folders and documents with versions, drawings with revisions and sheet measurements, schedule tasks with links, baselines and critical path, transmittals, meetings, tasks, incidents, photos, correspondence, forms, timesheets, action plans, estimating, bidding (with the bidder portal), prequalification, company reports, project report |
| Store | journal, snapshots, blobs, credentials on disk; scheduled backups, restore |
| Web UI | a set of pages for every tool above (list, detail, forms, files, comments, CSV and PDF export where it applies); schedule with a Gantt chart, critical path and float; drawings with the sheet viewer (a Rust-to-WebAssembly module, with a server-drawn fallback); budget views, commitments, invoices and change management; estimating and bid leveling; the company directory with the team page (template, role and per-tool overrides); saved company reports; My account (password, API tokens); the interface in English, Spanish, French and Dutch |
| Outside companies | bidder portal: invited companies open an emailed link (no account), read the bid documents, ask questions and send a bid (`docs/bidder-portal.md`) |
| Server | signed session cookies, CSRF on every form, `POST /do/{command}`, `POST /api/v1/commands/{id}` and MCP over HTTP (`POST /mcp`) with a bearer token, email notifications through the company's own SMTP server |
| App | `serve`, `demo`, `run`, `commands`, `token`, `mcp`, `backup`, `restore`, `check`, `version` |
| Releases | a version tag builds Linux (static), macOS and Windows with the sheet viewer embedded; Windows is signed and macOS signed and notarized; a multi-architecture container image; archives with `SHA256SUMS`, build attestations and `install.sh` (`docs/deployment.md`). No release has been published yet. |
| Not yet | the remaining parts listed in `docs/gaps.md`; sharing documents by link, offline field mode and mobile apps, native Microsoft Project (.mpp) files, integrations with other software and electronic signing inside BuildKiosk; it has not been run on a real job site |

## Upcoming
1. Publish the first release (tag `v0.1.0`; the version in `Cargo.toml` and the tag must match).
2. The open parts of the tools already built, in the order of `docs/gaps.md`: inspections (4), drawings (5), schedule (6), financials (7), company reports (10), prequalification (11), languages (12).

## Progress log
- 2026-10-20: Schedule: resource levelling (preview and apply on the resources page; `schedule.level_plan`, `schedule.level`), and resources, limits and the working calendar read from Microsoft Project XML files on request.
- 2026-10-14: Releases: pushing a version tag runs the gate, builds the program for Linux (static), macOS and Windows with the sheet viewer embedded, checksums and attests the archives, and publishes a GitHub release; `install.sh` installs one after checking its checksum (`docs/deployment.md`). `buildkiosk version` added.
- 2026-10-16: Container image: the release workflow also builds a multi-architecture (Linux x86_64 and aarch64) image from the same static binaries and pushes it to `ghcr.io/sitesentrix/buildkiosk` on a tag (`contrib/docker/Dockerfile`, `docs/deployment.md`).
- 2026-10-20: Signed releases: the Windows program is signed with Azure Artifact Signing and the macOS programs with a Developer ID certificate and notarized (jobs in the `release` environment; unsigned with a warning when the secrets are missing), before the archives, checksums and attestations are made (`.github/workflows/release.yml`, `contrib/macos/`, `docs/deployment.md`).
- 2026-10-20: Financials: the budget can be grouped by section or by cost type as well as by division; cost codes carry a cost type (`cost_code.set_type`, `cost_code.set_types`).
- 2026-10-14: Bidder portal: invited companies open an emailed link (no account) to read the bid documents, acknowledge addenda, ask questions and send, change or withdraw a bid; bids stay sealed until bidding closes; the team creates, resends and cancels links from the package page and answers questions (design in `docs/bidder-portal.md`). Then: receipt emails, notices to the team, bidders attaching files to their bid, and an open bid link where companies ask to be invited and the team approves each.
- 2026-10-14: Languages: the interface in Spanish, French and Dutch as well as English (a picker at the foot of every page; pages translated on the way out from catalogs; a check that keeps the catalogs complete).
- 2026-10-14: Larger gaps: measuring paths and outlines on sheets, Primavera exchange (.xer) import, MCP resource subscriptions (also as an event stream over HTTP), backups pushed to S3-compatible storage; unembedded PDF fonts verified.
- 2026-10-13: Small items: estimate templates, files on action plans, prequalification questionnaire versions, repeat rules for month-end, quarterly and yearly.
- 2026-10-13: Company reports: saved reports with chosen columns, totals, a chart, CSV and PDF; the schedule exports as a project XML file.
- 2026-10-13: Bidding: bid form lines and alternates, scope checklist and leveling matrix, addenda with acknowledgements, bid files, required bid bonds.
- 2026-10-13: Estimating: company price catalogue and assemblies, takeoff totals from sheet measurements, estimate versions and comparison, alternates, bids as lines.
- 2026-10-13: Prequalification: scored questionnaires, files, warnings before expiry, standing shown on bids and commitments, an optional rule requiring a current prequalification.
- 2026-10-13: Transmittals: items linked to documents and drawings with the version sent, recipient acknowledgement of receipt, cover sheet with a receipt block.
- 2026-10-13: Meetings: repeat a meeting on a schedule, carry open actions forward, make a task from an action, printable sign-in sheet.
- 2026-10-13: Incidents: injury log with rates (page, PDF, CSV), serious reports notify the tool admins, printable incident report, private incidents.
- 2026-10-13: Tasks: subtasks, repeating tasks, a "My tasks" page across projects, and links to the RFI or meeting a task came from.
- 2026-10-13: Operations: scheduled backups while the server runs (`serve --backup-dir`, keeping the newest N) and a `restore` command that checks the backup first and moves the old data aside.
- 2026-10-13: MCP: tool results also carry `structuredContent`; project documents are resources (list in pages, templates, read).
- 2026-10-13: Financials: configurable budget views (columns, grouping by division), a forecasting method per project (budget, committed cost, percent complete), a signed-contract workflow on commitments with an optional approval rule, and budget import from an uploaded CSV file that can add the missing cost codes.
- 2026-10-13: a refused save sends the person back to the form with what they typed (kept on the server for ten minutes, never passwords).
- 2026-10-13: PDF export (own writer, no dependency) for RFIs, submittals, daily logs, meetings, transmittals, correspondence, estimates, bid leveling, action plans, forms, invoices, budget, drawing log, schedule, project report and work-item lists.
- 2026-10-13: My account page: change your own password, create and revoke API tokens.
- 2026-10-13: MCP over HTTP (`POST /mcp` with an API token) on the running server.
- 2026-10-13: the project sidebar is grouped into collapsible sections (project management, quality and safety, plans and files, financials, preconstruction, resources) and only lists tools the person can read.
- 2026-10-13: directory: edit companies; add and edit permission templates (level per tool).
- 2026-10-13: prequalification (company-level records, review, lapse warnings). Every Procore tool now has a first slice.
- 2026-10-13: estimating (priced lines by cost code, markups, issue, put in the budget, CSV).
- 2026-10-13: bidding (bid packages, invited companies, recorded bids, leveling against the budget, award with a draft commitment).
- 2026-10-13: action plans (outline of sections and items, owners and dates, start, tick off, complete).
- 2026-10-13: action plans, more: approvers who must approve before a plan starts, comments on items, sign-off, company plan templates.
- 2026-10-13: timesheets, more: pay rates, overtime rules, crew entries, labour cost report, posting labour to direct costs.
- 2026-10-13: forms, more: conditional questions, typed-name signatures, template copy across projects, recurring forms, cross-form report.
- 2026-10-13: correspondence, more: configurable types with their own numbering, links to RFIs and documents, letter print, incoming mail and pasted emails.
- 2026-10-13: photos, more: tags to RFIs, observations and punch items, slideshow, album zip, upright thumbnails, picture metadata.
- 2026-10-13: documents, more: restricted documents and in-page previews; schedule, more: four more constraints and company holiday lists.
- 2026-10-13: financials, more: budget PDF follows the view, default budget view per person, signatures on the prime contract and change orders.
- 2026-10-13: MCP prompts and drawing/photo resources; back-up-now page, second backup copy, single-company restore.
- 2026-10-13: RFI ageing report columns, company injury log, task calendar, meeting templates.
- 2026-10-13: bid scoring and bond check, catalogue prices by date and supplier, saved crews.
- 2026-10-13: named schedule baselines, submittal links on correspondence, photo comparison.
- 2026-10-13: owner decision: no hosting for now (design parked in docs/hosting.md); monetise through support, training and setup of self-hosted installs.
- 2026-10-13: portfolio table on the Projects page and `report.portfolio`.
- 2026-10-13: project report across tools (open and past-due counts) and a "waiting on me" list on the project home; `report.summary` and `report.waiting_on_me` queries.
- 2026-10-13: timesheets (hours by day and cost code, weekly submit, approve and send back, CSV).
- 2026-10-13: forms (project-defined templates, typed questions, fill-in records, required answers, submit).
- 2026-10-13: correspondence (draft, send, replies, close, private, files).
- 2026-10-13: photos (gallery by album, multi-file upload, picture-only validation, inline display of JPEG/PNG/GIF/WebP only).
- 2026-10-13: incidents (severity, recordable, investigation fields on the shared workflow).
- 2026-10-13: tasks (the shared punch-list workflow: assignees, review loop, comments, files, CSV).
- 2026-10-13: meetings (agenda, attendance, minutes, action items, issue with notifications, files).
- 2026-10-13: transmittals (draft, send, recipients notified, items, files, private).
- 2026-10-13: files on commitments (signed contract, certificates), printable payment applications and CSV export for both invoice kinds.
- 2026-10-13: budget forecasting (forecast to complete, projected cost, over/under), budget snapshots with a compare view, locking of the original budget.
- 2026-10-13: schedule: tasks in working days, summary tasks, finish-to-start links with lag, critical path and float, recalculate, baseline, import, Gantt chart (SVG, no JavaScript), CSV.
- 2026-10-13: drawings: sheet log with discipline and area, revisions with files, log import, CSV.
- 2026-10-13: documents: folders (private folders with a people list), files with a version history, search, upload and download pages.
- 2026-10-13: operations: `serve --secure-cookies` (Secure cookies, HSTS), `backup` and `check` commands, `docs/deployment.md`.
- 2026-10-13: MCP server (`buildkiosk mcp`): every command is a tool, runs as a person; `docs/api.md`.
- 2026-10-13: Commitment invoices — payment applications against commitments with retainage; owner and commitment invoices share one implementation.
- 2026-10-12: Owner billing — prime contract with retainage, prime change orders, owner invoices (payment applications) and the billing roll-up.
- 2026-10-12: Change management — change events, commitment change orders, budget changes; the budget shows revised budget and pending exposure.
- 2026-10-12: Financials, first slice — budget lines and import, commitments with approval, direct costs, budget roll-up; amounts are exact money with checked arithmetic.
- 2026-10-12: Cleanup — punch list and observations run one shared workflow (`WorkItem`, `WorkCore`); stored data unchanged.
- 2026-10-12: Inspections — templates and starter set, answered checklists, punch items from failed lines.
- 2026-10-12: Observations — category, date observed, corrective action, review loop, files, notifications.
- 2026-10-12: Punch list — assign, work done, review, send back, close, reopen; comments, files, notifications, CSV.
- 2026-10-12: Daily log — weather, manpower, equipment, deliveries, visitors, notes; draft, submit, reopen.
- 2026-10-12: RFI notifications.
- 2026-10-13: Email — notifications are emailed through the company's own SMTP server (settings and test send on an admin page, per-person opt-out, links back to the app, nothing sent twice).
- 2026-10-13: Sheet viewer — PDF pages drawn on the server, click-to-place markups kept per revision, revision overlay; no JavaScript or WASM needed.
- 2026-10-13: Schedule: constraints, resource loading with limits, Microsoft Project XML import, drag editing in the chart, days off and baseline drawn on the chart, printable look ahead.
- 2026-10-13: Schedule: start-to-start, finish-to-finish and start-to-finish links, a working calendar per project (weekdays and holidays) used for all date maths, and a look-ahead page.
- 2026-10-13: Measuring on sheets: per-page scale, lengths and areas worked out by the engine, live readout in the browser viewer; revision, page and compare without reloading; module gzipped on the wire.
- 2026-10-13: Browser sheet viewer in WebAssembly (`crates/viewer`): smooth pan and zoom, sharp redraw, click-to-place markups through the same command; built by `cargo xtask viewer`, optional, with the server-drawn viewer as fallback. First client code in the project (owner-approved exception, AGENTS.md).
- 2026-10-13: Documents (multi-file upload, zip download, folder moves, deleted files page) and drawings (sheet-set upload, publish a revision, sheets linked to RFIs and work items).
- 2026-10-13: Item 4 finished — repeating and signed inspections, files on checklist lines, section scores, files on daily logs, photo thumbnails.
- 2026-10-13: Inspections (template edit and copy, printable checklist), observations (category list, trends), daily log (copy previous day, weekly summary, submit notification).
- 2026-10-13: Punch list, observations, incidents and tasks — CSV import, filters by assignee, location and type, grouping, filtered PDF for site walks.
- 2026-10-13: Submittals finished — register import from an uploaded CSV file, reviewer groups, a submittal log report (page and PDF).
- 2026-10-13: Directory and permissions pages finished — per-tool overrides per team member, project settings, company settings, merging duplicate companies.
- 2026-10-13: RFIs — files on individual responses (multipart post, up to 10 files, download with the RFI's files).
- 2026-10-12: Submittals round 2 — ordered reviews, register import/export, specification sections, files on reviews, in-app notifications.
- 2026-10-12: Submittals — review workflow with revisions, files, history.
- 2026-10-12: Directory page (people, companies, templates), admins set passwords, edit/deactivate/remove people.
- 2026-10-12: RFIs — files (25 MB, served as downloads), change history, distribution list, CSV export.
- 2026-10-12: M1 — sign-in, projects, team and RFIs work end to end in the browser.
