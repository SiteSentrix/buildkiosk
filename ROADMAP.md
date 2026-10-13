# BuildKiosk roadmap

_Status: 2026-10-13. Stage: every scoped tool has a first slice (see the progress log). Numbers are estimated, not measured._

Scope (owner decision, 2026-10-12): Procore's tools only; BuildKiosk-only extras are parked (see AGENTS.md).
The ranked work list is [`docs/gaps.md`](docs/gaps.md).

## Where we are
| Area | State |
| --- | --- |
| Engine (commands, permissions, journal) | company setup, directory, permission templates, projects, project team, cost codes, RFIs, submittals, specification sections, notifications, daily logs, punch list, observations, inspections, budget, commitments, direct costs, change events, commitment change orders, budget changes, prime contract, prime change orders, owner invoices, commitment invoices, folders and documents with versions, drawings with revisions, schedule tasks with links and critical path, transmittals, meetings, tasks, incidents, photos, correspondence, forms, timesheets, action plans, bidding, estimating, prequalification, project report |
| Store | journal, snapshots, blobs, credentials on disk |
| Web UI | schedule (task table with a Gantt chart in inline SVG, critical path, float, baseline, import, CSV); drawings (sheet log with filters, import, revisions, files, CSV); documents (folders, private folders, upload, versions, search, download); budget (rolled up by cost code), commitments (schedule of values), direct costs; inspections (list, checklist, form, templates); observations (same views as the punch list, plus category, date observed, corrective action); punch list (list, detail, form, comments, files, CSV); daily log (list by month, detail, form with crews, equipment, deliveries, visitors); submittals (list, detail, form, reviews, close, resubmit, files); sign-in, first-run setup, projects, project home, team, RFIs (list, detail, new, edit, respond, open, close, reopen, delete, files, history, distribution list, CSV export) |
| Directory | company directory page: people, companies, permission templates (view), add forms, admin sets passwords |
| Server | signed session cookies, CSRF on every form, `POST /do/{command}`, `POST /api/v1/commands/{id}` (bearer token) |
| App | `serve`, `demo`, `run`, `commands`, `token`, `mcp`, `backup`, `check` |
| Not yet | file attachments, submittals and every other tool |

## Upcoming
1. Finish RFIs: files on responses.
2. Next tools in the order of `docs/gaps.md` (then the financial remainder; financials still to do: forecasting, budget snapshots).

## Progress log
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
- 2026-10-12: Submittals round 2 — ordered reviews, register import/export, specification sections, files on reviews, in-app notifications.
- 2026-10-12: Submittals — review workflow with revisions, files, history.
- 2026-10-12: Directory page (people, companies, templates), admins set passwords, edit/deactivate/remove people.
- 2026-10-12: RFIs — files (25 MB, served as downloads), change history, distribution list, CSV export.
- 2026-10-12: M1 — sign-in, projects, team and RFIs work end to end in the browser.
