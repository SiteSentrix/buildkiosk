# Prequalification links

_Status: 2026-10-20. Built. Follows the design and security checklist of the bidder portal (`docs/bidder-portal.md`);
this page says only what is different._

## What it is

A company's contact opens `/prequal/<link>` with no account and fills in the company's own prequalification record:
the questionnaire answers, insurance and licence dates, experience modification rate, bonding capacity, documents,
and a message to the reviewers. Putting the record forward locks the answers and tells the Prequalification admins.
Only the reviewers approve, reject, score or set the approval date.

## How a link works

- The team gives a contact a link with `prequal.invite` (company, email, optional last day). The record is made if
  the company has none. `prequal.resend_invite` makes a new link and the earlier one stops working;
  `prequal.revoke_invite` cancels it. Up to 20 links per record.
- The link is `/prequal/<company>.<number>.<generation>.<signature>`. The signature is an HMAC of the server's slug,
  the company, number and generation under the server's secret, so nothing stored (state, journal, backups, exports)
  can open it. The journal holds the number, address, generation, last day and sent/cancelled times, never the link.
- The server emails the link when outgoing email and its public address are set up (the same 20-second pass that
  sends bid links), then records it with `prequal.invite_sent` (system only). Without email the team page shows the
  link to copy (`/prequalification/<company>/links/<number>`).
- Every page and post checks the signature, then the engine's `prequal.portal_*` commands (system only, so a person,
  API token or agent can't call them) check the invitation again: the right company, the generation the server
  verified, not cancelled, not past its last day. Every failure looks the same.

## What the company can and cannot do

| | Pending or rejected | Submitted | Approved |
| --- | --- | --- | --- |
| Answer, change dates and figures | yes | no | no |
| Put the record forward | yes (every question answered) | no | no |
| Attach a document | yes | yes | yes (the admins are told) |
| Remove a document | only ones it attached | no | no |
| Leave a message | yes | yes | yes (the admins are told) |

A renewal works by the company attaching the new certificate and leaving a message; the team reads it and updates
the dates. A company can never change what the reviewers decided, see their notes or score, or reach another
company's record.

## Warnings to the company

The daily pass that warns the admins before an approval, insurance or licence lapses is joined by one for the
company's own contact: for each of 60, 30 and 7 days before, and once lapsed, `prequal.company_due` lists what is due
for companies with a link that was emailed and still works, the server emails the contact (with the link, to send the
renewal), and `prequal.company_reminded` records it so it goes once. A company with no working link gets nothing; the
admins are warned as before.

## Security notes

Same as the bidder portal: rate limit per link, a form token per link (never a cookie), `Referrer-Policy: no-referrer`,
`Cache-Control: no-store`, `X-Robots-Tag: noindex`, uploads capped at 25 MB, no JavaScript. The email says the link
is for one person only. Not covered: forwarding of the email (the link is the credential), and files are not scanned.
