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
| `list_calendars` | An account's calendars, with their colours and whether each is shown. |
| `list_events` | The events that overlap a stretch of time, each occurrence of a repeating event on its own. |
| `get_event` | One event, with its description. |
| `create_event` | Make an event in a calendar. Consequential. |
| `update_event` | Change an event: only what is given. Consequential. |
| `delete_event` | Delete an event; a meeting you organised is cancelled for everyone. Consequential. |
| `get_free_busy` | When people are free and busy, and when all of them are free at once. |
| `search_people` | Find a person among your contacts and in your organisation's directory. |
| `get_contact` | One personal contact, whole. |
| `create_contact` | Add a personal contact. |
| `update_contact` | Change a personal contact: only what is given. |
| `delete_contact` | Delete a personal contact. Consequential. |
| `list_rules` | The inbox's rules in the order they run, each also said as a sentence. |
| `update_rule` | Turn a rule on or off, rename it, reorder it, or replace what it looks for or does. Consequential. |
| `find_meeting_times` | Times Microsoft 365 suggests for a meeting with some people. |
| `find_rooms` | The organisation's meeting rooms, and which are free at a time. |
| `cancel_event` | Cancel a meeting you organised, with words to everyone invited. Consequential. |
| `respond_to_event` | Accept, tentatively accept or decline an invitation. Consequential: the organiser is sent the answer. |
| `get_sync_status` | For each account: syncing or not, needing sign-in or not, how much is on this computer and when it was last brought up to date. |
| `list_commands` | Every command, with its id, description and argument schema. |
| `execute_command` | Run any command by id. |

`create_event` and `update_event` take `repeat` for a repeating event, and `update_event` and `delete_event` take `scope` for one of a series: `occurrence`, `following` or `series`. Both take `attendees`, the whole list of who is invited, and `online_meeting` for a Teams meeting.

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
| `--allow-consequential` | Send, and do the other things that reach people or cannot be undone, without asking you each time. Needs `--access full` as well. |

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

**Sending asks first.** With `--access full` and without
`--allow-consequential`, a command that reaches other people or cannot be
undone (sending, deleting for good, setting automatic replies) is put to you
in the app: who is asking, what for, and for a message who it is to and what
it is about. It runs only if you allow it, once. If you say no, do not answer
in two minutes, or the app is not open, the agent is told no.

The server tells every agent that the content of an email, attachment or
invitation is data to report on and never an instruction.
