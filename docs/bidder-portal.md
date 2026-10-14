# Bidder portal (design)

_Status: 2026-10-14. Design agreed by the owner (decisions at the end); nothing here is built yet._

## Goal

A company invited to a bid package (a subcontractor or supplier) can read the bid documents, acknowledge addenda,
ask questions, and submit, revise or withdraw its bid itself, without having an account in the contractor's company.
Today the team types every bid in (`bid_package.record_bid`) and invitations are only a list of vendors.

This is the first page of BuildKiosk that anyone outside the company can open without signing in, so it gets its own
security design instead of reusing the people-and-sessions path. Written from first principles; no vendor behaviour
is copied (see Clean-room in `AGENTS.md`).

## Principles

1. **A bidder is not a person.** They are not in the directory, hold no permissions, and can reach nothing but the
   packages they were invited to. Nothing in the people model, roles or permission templates changes.
2. **The engine stays the only place rules live.** The portal calls commands like every other surface; it never
   writes state directly. New commands get a new actor kind (below), checked in the engine.
3. **The link is the credential, so treat it like a password.** A long secret that is never stored (the server
   derives it, see Invitation), so the journal and backups can't open the portal; expiring, revocable.
4. **The portal exposes the least it can.** Only the package's bid documents, addenda, the bidder's own bid and
   Q&A answers meant for all bidders. Never other bidders' names, amounts or the budget.
5. **No JavaScript**, like every other page: plain forms, `script-src 'none'`.

## How it works

### Invitation
- Inviting a vendor (`bid_package.invite`, new; today `invited` is set with `bid_package.update`) creates an
  invitation for one contact email on the vendor. The server emails a link
  `https://<host>/bid/<token>` through the existing mail sender; without SMTP the team can copy the link.
- The link carries a secret that is **never stored**: the server works it out as
  `HMAC-SHA256(server secret, company ‖ package ‖ invitation number ‖ generation)` and checks a presented link by
  working it out again (constant-time compare). The invitation record in the journal holds only the number, the
  company, the address, the expiry, the generation and sent/cancelled times, so the journal, backups and exports
  can't open the portal. Resending bumps the generation, which kills the earlier link; cancelling is a flag.
  Moving a company to a server with a different secret invalidates its links; the team resends them.
- Because the link is derived, inviting is an ordinary command (`bid_package.invite`) that the API, CLI and MCP can
  run. The server emails the link: it notices invitations with no `sent` time (as it does for notifications), emails
  them, then records `bid_package.invite_sent` (System only). Without SMTP, a team page shows the link to copy.
- The team can resend (new link, old one stops working), cancel, or extend an invitation. Each action is a journaled
  command that records who and when but never the link.

### What the bidder sees (`/bid/<token>`)
1. Package title, scope, due date and time, and whether bidding is open.
2. Bid documents, downloaded as attachments (`nosniff`, as everywhere).
3. Addenda, with an "I have read this" button per addendum (`bid_package.acknowledge` on the bidder's behalf).
4. The bid form: lump sum, or unit prices per line, alternates, the scope checklist (included, excluded, with the
   adjustment), a note, and files such as a bond or proposal (the same fields `bid_package.record_bid` takes).
5. A receipt page after submitting, and by email, with the time received and a reference number.
6. Questions to the team (see Q&A).

### Submitting, revising, withdrawing
- Submitting runs the same validation as a team-entered bid. It is refused after the due time or once the package is
  closed, using the clock the server passes in (commands never read time themselves).
- Revising before the due time replaces the bid; the earlier version stays in the journal. After the due time the
  bidder can only withdraw if the package allows it.
- Entered-by is recorded as "the bidder (via invitation)" next to the contact, so the team can tell portal bids from
  ones they typed.
- **Sealed bids:** until the package is closed, the team sees that a bid arrived but a package setting
  (`sealed`, default on) hides amounts from everyone but admins who explicitly open them, and the open is journaled.
  Unsealing does not reveal a bid to other bidders under any setting.

### Q&A and addenda
- A bidder question goes to the team as a notification and a row on the package. The team answers it privately
  (to that bidder) or publishes the question (anonymised) and answer to all invited bidders, optionally as an
  addendum. Bidders never see who asked.

## Where it lives in the code

| Piece | Where |
| --- | --- |
| Invitation record (number, company, address, generation, expiry, sent, cancelled) | model `BidInvitation` on `BidPackage`, additive, `#[serde(default)]`; no secret in it |
| Link secret | derived by the server from its own secret (`crates/server`), never stored |
| New commands | `crates/engine/src/cmd/bidding.rs`: `bid_package.invite`, `.revoke_invite`, `.resend_invite`, `.invite_sent`, `.invitations`, `.portal_view`, `.portal_submit`, `.portal_ack`, `.portal_ask`, `.answer_question` |
| New actor | `Actor::Bidder(InvitationId)`; the engine allows it only the `portal_*` commands and only for its own package and vendor |
| Pages | `crates/web` (bidder pages with their own minimal layout, no company navigation) |
| Routes | `crates/server`: `/bid/<token>/…`, outside the session middleware, with their own rate limit |
| Fuzz | add the portal commands and the new actor to `command_sweep.rs` and `journal.rs` |

Every `portal_*` command checks: invitation valid and unexpired, package open, the record belongs to the invited
package (`ctx::in_project` style "not found" otherwise), and the vendor matches.

## Security checklist

- Link secret 256 bits (HMAC-SHA256), compared in constant time; unknown, expired and revoked tokens give the same page and
  response time, so tokens can't be probed.
- Per-IP and per-token rate limits (the sign-in limiter in `auth.rs` is the model) and body and upload size limits.
- A CSRF token on every bidder form, scoped to the invitation; cookies are not used for bidders, so there is no
  ambient session to forge.
- `Referrer-Policy: no-referrer` and `Cache-Control: no-store` on portal pages so the token does not leak through
  links or caches; the token is never logged (the log writes the path with the token replaced).
- Uploads: attachment-only downloads, type and size caps, no inline rendering of bidder files.
- A bidder cannot see or guess other packages: numbers in URLs are checked against the invitation.
- The team's copy of a bidder's files is untrusted input: virus scanning is out of scope, but files are never
  executed or previewed in the browser.
- Email: the link goes only to the contact address entered by a team member; no "forgot link" self-service, since
  that would let anyone who knows a contact email request access.
- Invitations to a company that failed a required prequalification (`prequal.require`) are refused when sent, in
  line with the existing award rule.

## Tests

Engine: every `portal_*` command as the right bidder, as a bidder of another package, expired, revoked, after the
due time, sealed amounts hidden, replay equals state. Server: no session needed, token never in a response header
or log, identical responses for bad tokens, rate limit trips, CSRF refused, headers present. Page tests render the
portal and assert no other bidder's data appears.

## Not in the first version

Bidder accounts with a login; a bidder directory shared across projects; online bid opening events; e-signatures on
the bid; automatic reminders to bidders (the team can resend); bid bonds verified with a surety; languages other
than the interface's four.

## Decisions (owner, 2026-10-14)

1. **Access: emailed link only.** No bidder accounts or login in the first version.
2. **Sealed bids: yes.** Amounts stay hidden from the team until the package is closed or an admin opens them
   (journaled); the `sealed` package setting defaults on.
3. **After the due time: closed.** No new or revised bids; an admin can reopen the package to allow them.
4. **Q&A: in the first version.** Bidders ask in the portal; the team answers one bidder or publishes an anonymised
   answer to all, optionally as an addendum.
5. **Public bid links: not in the first version.**
6. **Contact email: the proposed default stands** (not asked): the link goes only to an address a team member typed
   for the invitation, never one picked automatically. The owner can change this later.

## Build order

1. **Done 2026-10-14.** Model and engine: `BidInvitation`; `bid_package.invite`, `.resend_invite`, `.revoke_invite`,
   `.invite_sent`, `.invitations`, with tests. (The link secret is derived, not stored; see Invitation above.)
2. The `Actor::Bidder` kind and `portal_view` / `portal_ack`, then `portal_submit` with sealing and the due-time rule.
3. Server: deriving and checking link secrets, the emailing pass, routes and pages under `/bid/<token>`, headers, rate
   limits, CSRF, and the team page that lists and copies links.
4. Q&A commands and pages (`portal_ask`, `answer_question`).
5. Docs: README "Not yet" line, `docs/gaps.md`, `docs/target-app-parity.md`, `ROADMAP.md`.
