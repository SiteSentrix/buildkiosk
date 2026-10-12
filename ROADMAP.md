# BuildKiosk roadmap

_Status: 2026-10-12. Stage: M1 (web app skeleton). Numbers are estimated, not measured._

Scope (owner decision, 2026-10-12): Procore's tools only; BuildKiosk-only extras are parked (see AGENTS.md).
The ranked work list is [`docs/gaps.md`](docs/gaps.md).

## Where we are
| Area | State |
| --- | --- |
| Engine (commands, permissions, journal) | company setup, directory, permission templates, projects, project team, cost codes, RFIs |
| Store | journal, snapshots, blobs, credentials on disk |
| Web UI | sign-in, first-run setup, projects, project home, team, RFIs (list, detail, new, edit, respond, open, close, reopen, delete) |
| Server | signed session cookies, CSRF on every form, `POST /do/{command}`, `POST /api/v1/commands/{id}` (bearer token) |
| App | `serve`, `demo`, `run`, `commands`, `token` |
| Not yet | MCP server, file attachments, submittals and every other tool, email notifications |

## Upcoming
1. Finish RFIs: attachments, change history, export.
2. Next tools in the order of `docs/gaps.md` (submittals, daily logs, punch list, …).

## Progress log
- 2026-10-12: M1 — sign-in, projects, team and RFIs work end to end in the browser.
