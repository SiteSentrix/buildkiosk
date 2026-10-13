# Hosting BuildKiosk for other companies

Status: design, 2026-10-13. Nothing here is built yet. Owner direction: BuildKiosk will be offered as a hosted service
to companies that want it, as one way to fund the project. The self-hosted single-company server stays as it is and
stays the default; hosting is an extra mode of the same binary.

## What exists

- Each company is its own state, journal, blob store and credential store under `companies/<slug>`; nothing reads
  across companies (`docs/architecture.md`, Tenants).
- One running server opens one company (`App` holds a single `tenant`). `check`, `backup` and `restore --company`
  already work on a directory holding several companies.

## What hosting needs

1. **One server, many companies.** `App` holds a map of slug → company instead of one, each behind its own lock, so a
   slow request in one company never holds up another. Today's single global lock becomes one lock per company.
2. **Choosing the company for a request.** By hostname: `acme.example.com` is company `acme`. The reverse proxy gives
   a wildcard name and certificate; the server maps the `Host` header to a slug and answers "no such company" (the
   same page for an unknown and a suspended name) otherwise. A server started with `--company SLUG` or on a directory
   with one company behaves exactly as now (no hostname needed).
3. **Sessions that cannot cross companies.** The session cookie is scoped to the company's hostname, and the signed
   cookie and CSRF token carry the company slug, so a cookie from one company is useless on another even if the
   names were mixed up. Sign-in rate limits are kept per company and per address.
4. **Provisioning.** `buildkiosk company create --slug acme --name "Acme Builders" --admin EMAIL` makes the company and
   prints a one-time sign-in link for its first administrator. Later, a platform administrator page (a separate
   sign-in, not a company login) lists companies and can suspend or reactivate one. Suspending refuses sign-in and
   API calls but keeps the data.
5. **Background work per company.** The mail sender, the prequalification reminders, the recurring-forms pass and the
   scheduled backups run for every company, one after another, so one company's failure is logged and skipped.
6. **Limits per company.** Optional caps on stored bytes and on people (users), checked where the data is written
   (uploads and `person.create`), so a plan can be enforced without a billing system. A company over its cap can
   still read and export everything.
7. **Backups and exit.** Backups already include every company; `restore --company` brings one back. Add a
   `company export` that writes one company's journal and files as a folder a customer can take away, because a
   hosted customer should be able to leave with their data.
8. **Outgoing email.** Today SMTP settings are per server. In hosting each company keeps its own sender settings
   (their own SMTP), or all use one platform sender with the company's name as the display name.

## Isolation rules that must hold (and get tests)

- A request for company A never touches company B's state, files or credentials; a test signs in to two companies
  and tries every route with the other company's cookie, token and ids.
- Uploaded files are looked up under the company's own blob store only; a hash that exists in another company is not
  found.
- API tokens, MCP sessions and the command registry run against the company chosen by the hostname.
- Logs and errors never print another company's data; the platform admin page shows names, sizes and counts, never
  content.

## Order of work

1. Server serves many companies by hostname, with sessions scoped per company and the isolation tests (the large
   step: about 70 places in `crates/server` reach for the single company today).
2. `company create`, `company export`, suspend and reactivate from the command line.
3. Per-company background jobs and per-company limits.
4. Platform admin page; sign-up flow if wanted.

## Decisions that are the owner's

- **Pricing, plans and billing.** Not decided and not built. Plans can be enforced with the limits above; taking
  payment (a third-party integration) is parked under the scope rule in `AGENTS.md` until the owner decides.
- **Domain name and TLS** for the hosted service, and whether customers may use their own domain.
- **Terms of service, privacy policy and a data-processing agreement,** and where data is kept (region). These need
  the owner and a lawyer; the code only makes them possible (export, deletion, suspension).
- **Support promises** (backups, restore time, uptime) the owner is willing to make.
- **Deleting a customer's data** when they leave: how long it is kept after cancelling.
