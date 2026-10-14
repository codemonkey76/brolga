# The control channel

A running Brolga listens on a local socket, so scripts and other tools on the
same machine can run its commands. `brolga-cli` uses it whenever the app is
open.

## Where it listens

| System | Endpoint |
|---|---|
| Linux | The Unix socket `$XDG_RUNTIME_DIR/brolga/control.sock`, or `~/.cache/brolga/control.sock` where there is no runtime directory |
| macOS | The Unix socket `$TMPDIR/brolga/control.sock` |
| Windows | The named pipe `\\.\pipe\brolga-control-<user name>` |

On Linux and macOS the `brolga` directory is readable by its owner only, which
keeps other users of the machine out. Brolga makes it so, and will not use a
`brolga` that is a link, or that belongs to another user: it fails to start
the channel instead. A shared temporary directory such as `/tmp` is never
used on Linux.

## Who may call

Anything running as you. The channel does not ask who is calling, and a
caller can run any command with any arguments: there is no access level, no
asking first and no record of what was run. Those belong to
[the MCP server](mcp.md), and bind only an agent that has no other way to
run programs as you. A program that can open this socket has your mail.

The one exception is your side of an agent's question. The commands that
show a question whole and answer it (`app.confirmations.list`,
`app.confirmations.get`, `app.confirmations.answer`) run only from Brolga's
window, and are refused here with `window_only`.

## Windows

A pipe's name is in a namespace every user of the computer shares, so the
pipe is kept to you in two ways.

**Only its owner may open it.** Brolga makes the pipe with an access list
that allows its owner and nobody else. Another user of the computer cannot
connect to your Brolga. If you run Brolga as administrator, its pipe belongs
to the Administrators group, and a caller that is not also run as
administrator cannot open it.

**A caller checks that what answers is your Brolga.** Another user can still
make a pipe of that name before Brolga does, and answer on it. So when
Brolga starts it writes a key, 32 random bytes as 64 hex digits, to
`%LOCALAPPDATA%\Brolga\brolga-control-<user name>.key`, which only you can
read, and removes it when it closes. Before sending anything else, a caller
reads the key and asks for proof:

```json
{"id": 1, "method": "prove", "params": {"challenge": "<32 random bytes, as 64 hex digits>"}}
{"id": 1, "result": {"proof": "<64 hex digits>"}}
```

The proof is the SHA-256, in hex, of these bytes one after another: the
ASCII text `brolga control channel proof` and a line feed, the 32 bytes of
the key, and the 32 bytes of the challenge. A caller works it out for itself
and compares. If there is no key file, your Brolga is not running. If the
proof is wrong or refused, what answers is not your Brolga: hang up and send
it nothing. `brolga-cli` does this, and so should any tool of yours.

A challenge that is not 64 hex digits is refused with `no_proof`, as is
`prove` on Linux and macOS, where the socket's directory does this job.

If the name is already taken when Brolga starts, by something that cannot
give the proof, Brolga starts without the channel and tells you so. The
command line and agents cannot reach it until the other program is gone and
Brolga is started again.

## The protocol

One JSON object a line, in each direction. A line may be at most 1 MiB.

A request names a method and carries an `id` of the caller's choosing, which
comes back on the answer:

```json
{"id": 1, "method": "list_commands"}
{"id": 2, "method": "execute", "params": {"command": "app.version", "args": {}}}
```

`args` may be left out for a command that takes none.

An answer has either a `result` or an `error`:

```json
{"id": 2, "result": {"name": "Brolga", "version": "0.1.16"}}
{"id": 3, "error": {"code": "unknown_command", "message": "there is no command called `mail.nothing`"}}
```

| Error code | Meaning |
|---|---|
| `bad_request` | The line was not a request. `id` is null if it was not JSON either. |
| `too_large` | The line ran past 1 MiB. The connection is closed. |
| `unknown_command` | No command has that id. |
| `invalid_arguments` | The arguments do not fit the command's schema. The command did not run. |
| `failed` | The command ran and failed. |
| `no_proof` | `prove` was asked with a challenge of the wrong length, or where the channel has no proof to give. |
| `window_only` | The command is the person's own to run, in Brolga's window. It did not run. |
| `invalid_result` | The command gave back something its own schema does not allow. This is a bug in Brolga. |

`list_commands` returns every command with its id, title, description, kind,
and the JSON Schemas of its arguments and result.

A connection stays open for as many requests as the caller likes. Requests on
one connection are answered in order.
