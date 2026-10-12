# BuildKiosk roadmap

_Status: 2026-10-12. Stage: M1 (web app skeleton). Numbers are estimated, not measured._

Scope (owner decision, 2026-10-12): Procore's tools only; BuildKiosk-only extras are parked (see AGENTS.md).
The ranked work list is [`docs/gaps.md`](docs/gaps.md).

## Where we are
| Area | State |
| --- | --- |
| Engine (commands, permissions, journal) | company setup, directory, permission templates, projects, project team, cost codes, RFIs, submittals, specification sections, notifications, daily logs, punch list, observations, inspections |
| Store | journal, snapshots, blobs, credentials on disk |
| Web UI | inspections (list, checklist, form, templates); observations (same views as the punch list, plus category, date observed, corrective action); punch list (list, detail, form, comments, files, CSV); daily log (list by month, detail, form with crews, equipment, deliveries, visitors); submittals (list, detail, form, reviews, close, resubmit, files); sign-in, first-run setup, projects, project home, team, RFIs (list, detail, new, edit, respond, open, close, reopen, delete, files, history, distribution list, CSV export) |
| Directory | company directory page: people, companies, permission templates (view), add forms, admin sets passwords |
| Server | signed session cookies, CSRF on every form, `POST /do/{command}`, `POST /api/v1/commands/{id}` (bearer token) |
| App | `serve`, `demo`, `run`, `commands`, `token` |
| Not yet | MCP server, file attachments, submittals and every other tool, email notifications |

## Upcoming
1. Finish RFIs: files on responses, PDF export, email notifications.
2. Next tools in the order of `docs/gaps.md` (the financial tools).

## Progress log
- 2026-10-12: Cleanup — punch list and observations run one shared workflow (`WorkItem`, `WorkCore`); stored data unchanged.
- 2026-10-12: Inspections — templates and starter set, answered checklists, punch items from failed lines.
- 2026-10-12: Observations — category, date observed, corrective action, review loop, files, notifications.
- 2026-10-12: Punch list — assign, work done, review, send back, close, reopen; comments, files, notifications, CSV.
- 2026-10-12: Daily log — weather, manpower, equipment, deliveries, visitors, notes; draft, submit, reopen.
- 2026-10-12: RFI notifications.
- 2026-10-12: Submittals round 2 — ordered reviews, register import/export, specification sections, files on reviews, in-app notifications.
- 2026-10-12: Submittals — review workflow with revisions, files, history.
- 2026-10-12: Directory page (people, companies, templates), admins set passwords, edit/deactivate/remove people.
- 2026-10-12: RFIs — files (25 MB, served as downloads), change history, distribution list, CSV export.
- 2026-10-12: M1 — sign-in, projects, team and RFIs work end to end in the browser.
