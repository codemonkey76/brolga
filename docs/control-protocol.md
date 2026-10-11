# The control channel

A running Brolga listens on a local socket, so scripts and other tools on the
same machine can run its commands. `brolga-cli` uses it whenever the app is
open.

## Where it listens

| System | Endpoint |
|---|---|
| Linux | The Unix socket `$XDG_RUNTIME_DIR/brolga/control.sock` |
| macOS | The Unix socket `$TMPDIR/brolga/control.sock` |
| Windows | The named pipe `\\.\pipe\brolga-control-<user name>` |

On Linux and macOS the `brolga` directory is readable by its owner only, which
keeps other users of the machine out.

**Not yet restricted on Windows.** The pipe is created with Windows' default
permissions, which have not been checked against the "current user only"
requirement. Treat the Windows channel as unfinished until it is.

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
{"id": 2, "result": {"name": "Brolga", "version": "0.1.3"}}
{"id": 3, "error": {"code": "unknown_command", "message": "there is no command called `mail.nothing`"}}
```

| Error code | Meaning |
|---|---|
| `bad_request` | The line was not a request. `id` is null if it was not JSON either. |
| `too_large` | The line ran past 1 MiB. The connection is closed. |
| `unknown_command` | No command has that id. |
| `invalid_arguments` | The arguments do not fit the command's schema. The command did not run. |
| `failed` | The command ran and failed. |
| `invalid_result` | The command gave back something its own schema does not allow. This is a bug in Brolga. |

`list_commands` returns every command with its id, title, description, kind,
and the JSON Schemas of its arguments and result.

A connection stays open for as many requests as the caller likes. Requests on
one connection are answered in order.
