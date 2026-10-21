# Signing contracts in BuildKiosk

_Status: 2026-10-20. Built: commitments, prime contracts and prime change orders can be sent for signature with an emailed link; the other party signs on a page with no account. Left for later: below._

## What it does

Sending a contract for signature (`commitment.send_for_signature`, `prime_contract.send_for_signature`,
`prime_change.send_for_signature`) used to record only that it went out. Give it an `email` (and optionally `expires`,
the link's last day) and BuildKiosk makes a signing link as well:

- The server emails the link (when outgoing email and the public address are set up under Email settings), and the
  contract's page shows "Show link" so the team can copy it when email isn't set up.
- The other party opens `/sign/<link>`, reads the lines and the total, types their full name, ticks the box and signs.
  They can also decline, with a reason.
- The contract's signature record gets the typed name, the day, the moment (`signed_at`) and the version of the terms
  the signer saw (`signed_terms`). The team is notified, and the signer gets a receipt email.
- Paper still works: leave the email empty and record the signature by hand, as before.

## Safety

Same design as the bidder portal ([`bidder-portal.md`](bidder-portal.md)):

- The link is `<kind>.<id>.<generation>.<signature>`, an HMAC under the server's secret. Nothing stored (state, journal,
  backups) can open the page. Sending again makes a new generation (taken from the clock), so an old link never opens
  a newer sending, and a link stops working when the signature is taken back or after its last day.
- The `contract.portal_*` commands are system-only: a person, API token or agent can't call them. Each re-checks the
  contract and link, and every failure answers "this link is not valid".
- A signature is refused unless the signer sends the terms they were shown: if the contract changed after they opened
  the page, they must read it again. Changing a contract's lines while it is out for signature or signed clears the
  signature, as before.
- Per-link form token, rate limit, `no-store`, `no-referrer`, `noindex`, no JavaScript.

## What this is not

A typed name with a record of the time and the terms is a simple electronic signature. It is not a certified
signature service, and BuildKiosk does not claim it meets any particular law's requirements for a given contract;
check with your counsel. Digital certificates, witnesses, signing order and several signers per contract are not
built.

## Several signers

Sending a contract for signature can name more people (`others`: one email address on each line, up to six signers in all with the first). Each gets a link of their own. The contract is signed when every one has signed; a decline by anyone stops it for everyone. With `ordered` they sign one after the other as listed: a later signer's link opens and shows who has signed, but offers no form until the ones before have signed, and each is emailed only when it is their turn. Reminders go to whoever is next (or, in any order, everyone who hasn't signed). Every signature records the typed name, the moment and the terms version; the printed contract lists them all.

## The company's counter-signature

Sending a contract for signature can ask for the company to sign too (`countersign`). A person with standard access to the tool counter-signs from the contract's page (`commitment.countersign`, `prime_contract.countersign`, `prime_change.countersign`; as themselves, no typed name needed). The contract is signed when every signer and the company have signed, in whichever order: the company can sign first, and the signers' page lists the company's counter-signature with who is still to sign. The printed contract shows it.

## The printed contract

Each commitment, prime contract and prime change order has a PDF (`.../pdf`, linked from its signature card) with the lines, the total, and the record of the signature: who signed, the day, and for a signature through the link its moment and whether the contract still reads as it did when it was signed. It follows the same visibility rules as the contract's page.

## Reminders

A link that has been emailed and not used is followed by a reminder email three days after it was sent, and again every three days, three reminders at most (`contract.reminder_sent` records each; stopped by signing, declining, taking the contract back, or the link's last day).

## Left for later

- Several signers, signing in order, and counter-signature by the company on the same page.
