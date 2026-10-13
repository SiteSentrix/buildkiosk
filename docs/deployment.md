# Deploying BuildKiosk

_Status: written with the operations commands (M5.2). Covers a single self-hosted server._

BuildKiosk is one binary and one data directory. There is no database server to run.

## Run it

```
buildkiosk serve --data /var/lib/buildkiosk --bind 127.0.0.1:8080
```

The first visit to `/` opens first-run setup (company name and the first administrator). The data directory is
locked: a second `serve`, `run`, `backup` or `check` against the same directory refuses to start while the server
is running.

### systemd

```
[Unit]
Description=BuildKiosk
After=network.target

[Service]
User=buildkiosk
ExecStart=/usr/local/bin/buildkiosk serve --data /var/lib/buildkiosk --bind 127.0.0.1:8080 --secure-cookies
Restart=on-failure
StateDirectory=buildkiosk
ProtectSystem=strict
ReadWritePaths=/var/lib/buildkiosk
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

## HTTPS

The server speaks plain HTTP. Put a reverse proxy that terminates TLS in front of it (Caddy, nginx, a cloud load
balancer) and start the server with `--secure-cookies`. That marks the session and CSRF cookies `Secure` and adds
`Strict-Transport-Security`. Don't use the flag over plain HTTP: browsers will not send the cookies back and nobody
can sign in.

Caddy example:

```
buildkiosk.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Pages are served with `Content-Security-Policy: script-src 'none'`; the proxy needn't change or add headers.
Uploads are limited to 25 MB; set the proxy's body limit at least that high.

## What is on disk

```
instance.json                 the instance secret that signs cookies (keep private)
companies/<slug>/journal.jsonl every change, append-only (the source of truth)
companies/<slug>/snapshot.json a periodic copy of the state, to make start-up fast
companies/<slug>/blobs/       uploaded files
companies/<slug>/credentials.json password hashes, API tokens, sessions
```

## Backups

Stop the server, then:

```
buildkiosk backup --data /var/lib/buildkiosk --to /backups/2026-10-13
```

The destination must be new or empty and outside the data directory. The command copies everything, opens the copy,
replays each company's journal and prints the entry and row counts, so a backup that doesn't load fails loudly.
Move the backup off the machine (rsync, object storage). Keep several; they are small compared with the blobs.

Because the journal is append-only and fsynced before each change takes effect, a file-system snapshot (LVM, ZFS,
cloud volume snapshot) taken while the server runs is also consistent up to the last complete entry; an incomplete
final line is dropped with a warning when it is loaded.

`credentials` and `instance.json` are secrets: restrict who can read the backups.

## Restore

Stop the server, move the damaged directory aside, copy the backup to the data path and run:

```
buildkiosk check --data /var/lib/buildkiosk
```

then start the server. All sessions stay valid because `instance.json` is restored with the data.

## Check

`buildkiosk check --data DIR` replays every company from its snapshot and journal and prints the rows per table.
Run it after a restore, before an upgrade, or after a crash if you want reassurance.

## Upgrades

1. `buildkiosk backup` the data.
2. Replace the binary and restart.

Schema changes are additive, so a new build opens older data. If a release changes the schema in a way older builds
would misread, it bumps the schema number and older builds refuse to open the data rather than misreading it; to
go back to an older build after such an upgrade, restore the backup taken in step 1.

## Command line access

`buildkiosk run`, `token` and `mcp` open the data directory too, so they need the server stopped; to drive a running
server use the HTTP API with a token (see [`api.md`](api.md)).
