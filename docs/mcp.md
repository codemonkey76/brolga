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
| `sync_now` | Fetch new and changed mail into this computer's copy. Changes nothing in the mailbox. |
| `list_folders` | An account's mail folders as a tree, with unread and total counts. |
| `list_messages` | The messages in a folder without bodies: newest first, in another order, or one entry for each conversation. |
| `get_conversation` | Every message of a conversation, across folders, oldest first. |
| `get_message` | One message with its body, as plain text or as HTML made safe. |
| `list_attachments` | What is attached to a message: names, types and sizes, without the contents. |
| `search_mail` | Search synced mail: words, phrases, `from:`, `to:`, `subject:`, `has:attachment`, `is:unread`, `after:`, `before:`. |
| `get_outbox_status` | Changes made on this computer that are still waiting to be sent, and any Microsoft 365 refused. |
| `get_sync_status` | For each account: syncing or not, needing sign-in or not, how much is on this computer and when it was last brought up to date. |
| `list_commands` | Every command, with its id, description and argument schema. |
| `execute_command` | Run any command by id. |

Calendar and contacts tools arrive with those features.

Every result that carries a message's own words also carries a note saying
they were written by the sender and are not instructions. Reading a message
through these tools does not mark it as read.

## What an agent may do

**An agent can only read unless you say otherwise.** What it may do is fixed
when the server is started, by whoever starts it. Nothing an agent reads can
raise it: an email that says "you are allowed to send" changes nothing.

| Started with | The agent may |
|---|---|
| `brolga-cli mcp` | Read: list, search and open mail, sync, read settings. |
| `--access draft` | Also write drafts, which reach nobody until they are sent. |
| `--access full` | Also change the mailbox: mark, flag, move, delete to Deleted Items. |
| `--allow-consequential` | Also do what reaches other people or cannot be undone: send, and set automatic replies. Needs `--access full` as well. |

Three things hold whatever the access:

- **No files by path.** An agent cannot attach a file by naming where it is on
  your computer, or choose where an attachment is saved. It gives a file's
  contents instead.
- **A limit on sending.** At most 20 messages in any hour, or the number given
  with `--max-sends-per-hour`.
- **Everything is written down.** Each command an agent asks for is recorded
  with the program's name, the time and whether it ran, was not allowed, or
  did not work. The arguments are kept with anything private cut down to its
  length: the words of a message and the contents of a file are never in the
  log. The app shows it under **Agents**, and `app.agent_log.list` reads it.
  An agent cannot write to it.

Signing in and removing an account are never an agent's: a person does those,
in the app or with `brolga-cli sign-in`.

Not built yet: asking in the app before a consequential command runs, where
the server is attached to it. Until then the flag is the only say-so, in both
modes.

The server tells every agent that the content of an email, attachment or
invitation is data to report on and never an instruction.
