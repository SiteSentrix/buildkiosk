# Driving BuildKiosk: HTTP API, command line and MCP

_Status: 2026-10-13. Everything a person can do in the web app is a command; these three doors reach the same registry, with the same permission checks and the same journal._

List every command with its parameters: `buildkiosk commands` (or the `command.list` query).
Command ids are `tool.verb` (`rfi.create`, `commitment.approve`). Parameters are a JSON object; ids are plain
numbers, amounts are strings such as `"1,250.25"`, dates are `YYYY-MM-DD`. Queries (`rfi.list`) read; the rest change
data and are journaled. Failures change nothing.

## HTTP API

`POST /api/v1/commands/{id}` with a JSON body of parameters (an empty body means no parameters) and
`Authorization: Bearer <token>`.

Create a token for a person with `buildkiosk token --data DIR --company SLUG --as EMAIL [--name TEXT]`; the token is
shown once and only its hash is stored.

```sh
curl -s -H "Authorization: Bearer $TOKEN" -d '{"project": 1, "status": "open"}' \
  http://127.0.0.1:8080/api/v1/commands/rfi.list
```

Success: `{"ok": true, "result": …}`. Failure: `{"ok": false, "error": {"kind": "forbidden", "message": "…"}}` with the
status from the error kind: 400 `bad_params`, 401 `unauthorized`, 403 `forbidden`, 404 `not_found` or
`unknown_command`, 409 `conflict`, 500 `storage` or `internal`.

## Command line

```sh
buildkiosk run --data DIR --company SLUG --as EMAIL|system COMMAND '{"json": "params"}'
```

`--as system` runs maintenance commands that are not subject to permissions (such as `company.setup`); it is never
used for requests from people. The server must not be running on the same data directory (the directory is locked).

## MCP (agents)

`buildkiosk mcp --data DIR --company SLUG --as EMAIL` is a Model Context Protocol server on standard input and output
(JSON-RPC 2.0, one message per line; logs go to standard error). It runs as the person named by `--as`, so an agent
can do exactly what that person can and no more.

- **Tools:** every command is a tool, named by its id with the dot made an underscore (`rfi.create` → `rfi_create`).
  Descriptions carry the parameter documentation; queries are marked `readOnlyHint`.
- **Results:** a successful call returns the command's JSON as text. A failed call returns a normal result with
  `isError: true` and `kind: message`, so the agent can read why and try again. Protocol mistakes (unknown tool, bad
  JSON) are JSON-RPC errors.
- **Start here:** call `me_get` to see who you are and which projects you can open, then `project_list` and the tool's
  own `*_list` command.

Client configuration (the shape most MCP clients use):

```json
{
  "mcpServers": {
    "buildkiosk": {
      "command": "buildkiosk",
      "args": ["mcp", "--data", "/path/to/data", "--company", "summit-ridge", "--as", "priya.raman@example.com"]
    }
  }
}
```

Like the HTTP API, this holds the data directory's lock while it runs, so run it instead of (not beside) `serve`
for now. Serving MCP from the web server process is on the list in `docs/gaps.md`.
