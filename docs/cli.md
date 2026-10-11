# brolga-cli

Runs Brolga's commands from a shell and serves them to agents.

When the app is open, `brolga-cli` passes each command to it over the
[control channel](control-protocol.md). When it is not, the command runs in
`brolga-cli` itself.

```sh
brolga-cli commands                 # every command, with what it does
brolga-cli commands --json          # the same in full, schemas included
brolga-cli run app.version          # run a command that takes no arguments
brolga-cli run <id> --json '{...}'  # run a command with arguments
brolga-cli sign-in                  # add an account, in your browser
brolga-cli sign-in --device-code    # add an account with a code, from any device
brolga-cli mcp                      # serve the commands over MCP on stdio
brolga-cli mcp --connect            # the same, only through the running app
```

`run` prints the command's result as JSON on standard output. On failure it
prints the reason on standard error and exits with 1, or with 2 if `--json`
was not JSON.

## Signing in

`brolga-cli sign-in` signs in under Brolga's own app registration. Its
publisher is not verified yet, so an organisation that allows only apps it
has approved will refuse it. Such an organisation makes a registration of its
own and names it:

```sh
brolga-cli sign-in --client-id <application id> --tenant <tenant id or domain>
```

The registration must be a public client with the redirect URI
`http://localhost`, and for `--device-code`, with public client flows allowed.
The refresh token is kept in your system's secret store, never in a file.
