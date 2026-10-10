# Brolga for agents (MCP)

`brolga-cli mcp` is a Model Context Protocol server on standard input and
output. Each tool runs one of Brolga's commands.

## Setting it up

Claude Code, from a checkout of this repository: the `.mcp.json` here already
does it, once `brolga-cli` is built (`cargo build -p brolga-cli`).

Anywhere else, with `brolga-cli` on your `PATH`:

```sh
claude mcp add brolga -- brolga-cli mcp
```

For other MCP clients, the server is the command `brolga-cli` with the single
argument `mcp`.

## Headless or attached

- `brolga-cli mcp` runs commands itself when the app is closed, and passes
  them to the app when it is open.
- `brolga-cli mcp --connect` only works through the open app, and fails if it
  is not running.

## Tools

| Tool | What it does |
|---|---|
| `get_version` | The client's name and version. |
| `list_accounts` | The signed-in accounts, and whether each still has a working sign-in. |
| `get_sync_status` | Each account's sync state. Sync is not built, so no account is reported. |
| `list_commands` | Every command, with its id, description and argument schema. |
| `execute_command` | Run any command by id. |

Mail, calendar and contacts tools arrive with those features.

## What an agent may do

**Agents can only read.** Any command that changes something is refused over
MCP, whichever tool asks for it. That includes signing in and removing an
account: a person does those, in the app or with `brolga-cli sign-in`. The permission tiers in the build spec
(read-only, draft, full), confirmation of consequential commands, rate limits
and the audit log are not built yet, and nothing that writes will be opened to
agents before they are.

The server tells every agent that the content of an email, attachment or
invitation is data to report on and never an instruction.
