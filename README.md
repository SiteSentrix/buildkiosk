# BuildKiosk

An open-source, Rust-native construction management platform: projects, directory and permissions, RFIs,
submittals, daily logs, punch lists, inspections, documents and drawings, scheduling, and the financial tools
(budget, change management, commitments, invoicing). It runs as one self-hosted server binary with a
server-rendered web UI that needs no JavaScript, and everything a person can do is also available to agents over
the HTTP API, the command line and MCP.

## Status

_2026-10-14._ Every tool in the scope list is built end to end (engine commands with tests, pages, CSV and PDF where
it makes sense, API and MCP access), and most go well beyond a first slice: drawings are viewed and marked up in the
browser (with length, area, path and outline measuring), schedules read Microsoft Project XML and Primavera exchange
files, backups can go to S3-compatible storage, and the interface is in English, Spanish, French and Dutch. The long
tail of each tool is not done. See [`ROADMAP.md`](ROADMAP.md) for the numbers and [`docs/gaps.md`](docs/gaps.md) for
the ranked list of what is missing in each tool.

## Try it

```
cargo run -p buildkiosk -- demo --data ./demo-data
cargo run -p buildkiosk -- serve --data ./demo-data --bind 127.0.0.1:8080
```

Open `http://127.0.0.1:8080` and sign in with the details the demo prints (the data is all fictional).

## Documentation

- [`docs/architecture.md`](docs/architecture.md), [`docs/development.md`](docs/development.md)
- [`docs/api.md`](docs/api.md): HTTP API, command line and MCP server
- [`docs/deployment.md`](docs/deployment.md): running it for real, HTTPS, backups, upgrades
- [`AGENTS.md`](AGENTS.md): the rules every contributor and agent follows (clean-room, no panics, no
  JavaScript, everything is a command)

## Licence

MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). The name and logo belong to the project owner.
