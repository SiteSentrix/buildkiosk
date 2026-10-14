# Security policy

BuildKiosk is pre-release software that stores a company's project and financial records. Please report security
problems privately so they can be fixed before they are public.

## Reporting a vulnerability

Use GitHub's private reporting: open the repository's **Security** tab and choose **Report a vulnerability**
(`https://github.com/SiteSentrix/buildkiosk/security/advisories/new`). Please don't open a public issue or pull
request for a vulnerability.

Include what you found, the version or commit, the steps to reproduce it, and what an attacker could do with it.
Reports are answered on a best-effort basis; this is a small project and has no paid support.

## What is in scope

- Sign-in, sessions, API tokens, CSRF and rate-limiting bypasses.
- Permission bypasses: reaching another project's, another company's, or a private item's data, or running a command
  the actor's permissions don't allow.
- Injection (HTML, path, zip, import files), unsafe file serving, and uploads that escape their limits.
- Panics or crashes triggered by untrusted input (form posts, API and MCP messages, imported files). The project
  treats any reachable panic as a bug.
- Secrets reaching the journal, company state, logs or backups.

## Supported versions

There are no releases yet. Fixes go to `main`.

## For operators

[`docs/deployment.md`](docs/deployment.md) covers running BuildKiosk behind HTTPS, backups and upgrades. The server
has no built-in TLS; run it behind a reverse proxy that terminates HTTPS.
