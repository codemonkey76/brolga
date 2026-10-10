# Brolga

A native desktop client for Microsoft 365 mail, calendar and contacts, built
for Linux first, with Windows and macOS from the same code. It talks to
Microsoft Graph only, and everything it can do is also available to AI agents
through an MCP server, a command line and a local control channel.

**Status: milestone 0, foundations.** There is no sign-in, mail or calendar
yet. What exists is the skeleton the rest is built on.

- [The build spec](docs/build-spec.pdf)
- [Rules for contributors and agents](AGENTS.md)
- [Sources consulted](SOURCES.md)
- [brolga-cli](docs/cli.md), [MCP](docs/mcp.md), [the control channel](docs/control-protocol.md)

## Building

```sh
cargo test --workspace          # the core, with no window
cargo run -p brolga-cli -- commands

cd app && bun install && bun run tauri dev   # the window
```

The window needs Tauri's system libraries; on Linux that is WebKitGTK 4.1.
