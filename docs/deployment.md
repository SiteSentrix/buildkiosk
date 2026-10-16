# Deploying BuildKiosk

_Status: written with the operations commands (M5.2); container image added with the release workflow. Covers a single self-hosted server._

BuildKiosk is one binary and one data directory. There is no database server to run.

## Install a release

Each release on the [releases page](https://github.com/SiteSentrix/buildkiosk/releases) has an archive per platform
(Linux x86_64 and aarch64 as static builds that run on any distribution, macOS Intel and Apple silicon, Windows
x86_64), `SHA256SUMS` and `install.sh`. On Linux and macOS:

```
curl -fsSL https://raw.githubusercontent.com/SiteSentrix/buildkiosk/main/install.sh | sh
```

The script is short; read it first if you like (`install.sh --help`, `--dry-run`). It finds the newest release (pre-releases
included, or give `--version v0.1.0`), downloads the archive for the machine, **checks it against `SHA256SUMS` and
installs nothing if they differ**, and puts `buildkiosk` in `/usr/local/bin` as root or `~/.local/bin` otherwise
(`--prefix DIR/bin` to choose). On Linux with systemd, `sudo sh install.sh --service` also creates the `buildkiosk`
user, `/var/lib/buildkiosk` and the unit below (`--start` to enable and start it); it never overwrites an existing
unit. On Windows, download the `.zip`, unpack it and run `buildkiosk.exe`.

To check by hand: `sha256sum -c SHA256SUMS --ignore-missing`. Each archive also carries a GitHub build attestation, which
shows it was built by this repository's release workflow from a tagged commit:
`gh attestation verify buildkiosk-<version>-<target>.tar.gz --repo SiteSentrix/buildkiosk`.

To uninstall, delete the binary (and, for a service install, `systemctl disable --now buildkiosk`, the unit file and, if
you want the data gone too, `/var/lib/buildkiosk`: that is the company data, so take a backup first).

## Run it in Docker

Each release also publishes a container image for Linux x86_64 and aarch64 (the same static binary as the
archives) to `ghcr.io/sitesentrix/buildkiosk`. Tags are the version (`0.1.0`); `latest` follows stable releases only,
so while BuildKiosk is below 1.0 give the version.

```
docker run -d --init --name buildkiosk -p 127.0.0.1:8080:8080 -v buildkiosk-data:/data ghcr.io/sitesentrix/buildkiosk:0.1.0
```

Or with Compose:

```
services:
  buildkiosk:
    image: ghcr.io/sitesentrix/buildkiosk:0.1.0
    restart: unless-stopped
    init: true
    ports: ["127.0.0.1:8080:8080"]
    volumes: ["buildkiosk-data:/data"]
    # Behind an HTTPS reverse proxy (see HTTPS below), add the flag; the first three words are the image's default:
    # command: ["serve", "--data", "/data", "--bind", "0.0.0.0:8080", "--secure-cookies"]
volumes:
  buildkiosk-data:
```

- The image runs as an unprivileged user (uid 65532) and keeps all company data in the `/data` volume: that volume
  is the whole system, so back it up (below). A bind-mounted folder must be writable by uid 65532.
- It listens on port 8080 inside the container. Publish it on the loopback address as above and put the TLS proxy
  in front; don't expose plain HTTP to the internet.
- Other commands run the same way: `docker run --rm -v buildkiosk-data:/data ghcr.io/sitesentrix/buildkiosk:0.1.0 check --data /data`
  (stop the server first: the data directory is locked while it runs). Run it with `--init` (`init: true` in Compose): the server has no signal handler of its own, so as the container's first process it would ignore `docker stop` until Docker kills it after 10 seconds. Nothing is lost either way, because each change is fsynced to the journal before it takes effect.
- Verify an image the way you verify an archive: `gh attestation verify oci://ghcr.io/sitesentrix/buildkiosk:0.1.0 --repo SiteSentrix/buildkiosk`.
- To build one yourself from a release archive, see the comment at the top of `contrib/docker/Dockerfile`.

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

## Email

BuildKiosk sends each in-app notification as an email through your own SMTP server. A company admin sets it up
under My account → Set up outgoing email (`/admin/email`): host, port, encryption (STARTTLS, TLS or none for a
trusted local relay), user name and password, the sender address, and the public address of the server (used for
links in messages). The **Send a test email** button mails the admin's own address.

- The settings, including the SMTP password, are stored in `credentials.json` next to the journal, never in the
  journal. Back that file up like the rest of the data directory.
- The server checks for new notifications every 20 seconds. Turning email on does not send the backlog. A message
  that can't be sent is retried on the next pass; the log shows the reason.
- People turn their own emails off on My account. Only people who can sign in, are active and have an email
  address receive them.

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

### Backups while the server runs

Start the server with a backup folder and it takes backups on its own, without stopping:

```
buildkiosk serve --data /var/lib/buildkiosk --backup-dir /backups/buildkiosk --backup-every-hours 24 --backup-keep 7
```

The defaults are every 24 hours and keeping the newest 7. Each backup is a folder `backup-<unix seconds>` that was
replayed to prove it loads; one that fails or is interrupted is removed and the failure is logged. Requests wait only
while the small files (snapshot, credentials) are copied. The journal is copied up to the length it had at that
moment and the uploaded files after, so the backup is whole as of when it began. The folder must be outside the data
directory, and the server only ever deletes folders it named itself (`backup-<digits>`).

**A second copy.** Add `--backup-copy-to /mnt/offsite/buildkiosk` and every finished backup is also copied there
(another disk, or a share mounted from another machine, an NFS or SMB mount or an object-storage mount such as
rclone's), keeping the same number. If the second place is full or missing the backup itself is still kept and the
failure is logged. The copy is a plain folder you can restore from. For storage that can't be mounted, run your own
sync tool (`rclone`, `aws s3 sync`) from cron against the backup folder.

**Object storage.** Add `--backup-s3-bucket NAME` and every finished backup is also uploaded to storage that speaks
the S3 protocol (Amazon S3, MinIO, Backblaze B2, Cloudflare R2, Wasabi). Each file of the backup folder becomes one
object under `<prefix>backup-<seconds>/…`, signed with AWS Signature Version 4 and streamed (never held whole in memory).
The keys come from the environment of the server process, not the command line:

```
BUILDKIOSK_S3_ACCESS_KEY=… BUILDKIOSK_S3_SECRET_KEY=… \
buildkiosk serve --data /var/lib/buildkiosk --backup-dir /backups/buildkiosk \
  --backup-s3-bucket my-company-backups --backup-s3-region us-east-2 --backup-s3-prefix buildkiosk/
```

`--backup-s3-endpoint https://…` names the service (default `https://s3.<region>.amazonaws.com`; a MinIO on your own
network is `http://host:9000`), and `--backup-s3-virtual-host` addresses the bucket as `bucket.host` instead of
`host/bucket`. A key that may only write to the bucket is enough. Like the second copy, the upload is best effort: the
backup is kept locally either way, a failure is logged, and *My account → Backups* shows how the last upload went. Old
backups are not removed from the bucket; set a lifecycle rule there (expire after, say, 30 days). To restore, fetch
the folder (`aws s3 sync s3://bucket/buildkiosk/backup-1760000000 ./backup-1760000000`, or the same with `rclone`) and
run `buildkiosk restore` on it.

**Back up now.** Company admins open *My account → Backups* (`/admin/backups`) to see the backups kept, whether each
reached the second place, and a *Back up now* button.

## Restore

Stop the server, then:

```
buildkiosk restore --from /backups/2026-10-13 --data /var/lib/buildkiosk --replace
```

The backup is opened and replayed first: a backup that doesn't load changes nothing. With `--replace` an existing data
directory is moved aside to `/var/lib/buildkiosk.before-restore-<seconds>` (never deleted; remove it yourself once
you're sure), the backup is copied into place and the result is replayed again. Without `--replace` the data
directory must be missing or empty. It refuses to touch a directory a server is using, or a folder that isn't a
BuildKiosk data directory. Then start the server. All sessions stay valid because `instance.json` is restored with
the data.

### Restoring one company

If only one company went wrong, put just that company back and leave the others alone (stop the server first):

```
buildkiosk restore --from /backups/2026-10-13 --data /var/lib/buildkiosk --company acme
```

The backup is replayed first. The company's current folder is moved next to the data directory, to
`/var/lib/buildkiosk.before-restore-acme-<seconds>`, never deleted. A company that is not in the data directory (lost
altogether) is simply put back as of the backup.

## Check

`buildkiosk check --data DIR` replays every company from its snapshot and journal and prints the rows per table.
Run it after a restore, before an upgrade, or after a crash if you want reassurance.

## Building a release

**Publishing one** (maintainers): set `version` under `[workspace.package]` in `Cargo.toml`, commit it, then push a
tag of the same number with a `v`: `git tag v0.1.0 && git push origin v0.1.0`. The `Release` workflow
(`.github/workflows/release.yml`) refuses a tag that doesn't match the version, runs the whole gate (`cargo xtask ci`),
builds the viewer once and the program for five targets, starts each build (`version`, a demo company, `check`),
checksums and attests the archives, and creates the GitHub release with generated notes (marked a pre-release below
1.0.0). Nothing is published until a tag is pushed; the "Run workflow" button on the Actions tab builds everything
and keeps the archives as workflow artifacts without publishing. If a release is bad, delete the release and its tag on
GitHub and fix forward with a new version.

**By hand:** `cargo xtask viewer` then `cargo build -p buildkiosk --release`. The first builds the browser sheet viewer
(WebAssembly) and the second embeds it in the binary; without the first step the binary still works and sheets are
shown by the pages the server draws. Behind a proxy, allow `application/wasm` and serve `/static/viewer/*` as is.

## Upgrades

1. `buildkiosk backup` the data.
2. Replace the binary and restart.

Schema changes are additive, so a new build opens older data. If a release changes the schema in a way older builds
would misread, it bumps the schema number and older builds refuse to open the data rather than misreading it; to
go back to an older build after such an upgrade, restore the backup taken in step 1.

## Command line access

`buildkiosk run`, `token` and `mcp` open the data directory too, so they need the server stopped; to drive a running
server use the HTTP API with a token (see [`api.md`](api.md)).
