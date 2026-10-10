# Sources

Brolga is built only from public documentation and open standards. Every
outside document that informed the code is listed here, with what it was used
for. If you read something to write code, add it before the code is merged.

Nothing from Microsoft's own clients may be used: no decompiled or inspected
binaries, no captured traffic, no undocumented endpoints, no icons, sounds or
interface text. The full rules are in [AGENTS.md](AGENTS.md).

## Microsoft Graph (Microsoft Learn)

| Document | URL | Used for |
|---|---|---|
| Get a user | https://learn.microsoft.com/graph/api/user-get | `GET /me` and the properties returned by default, in the fake Graph server |
| Microsoft Graph throttling guidance | https://learn.microsoft.com/graph/throttling | The 429 status, the `Retry-After` header and the throttled error body, in the fake Graph server |
| Microsoft Graph error responses and resource types | https://learn.microsoft.com/graph/errors | The shape of an error body, and the meaning of 401, 404 and 503, in the fake Graph server |

## Model Context Protocol

| Document | URL | Used for |
|---|---|---|
| MCP Rust SDK (`rmcp` 3.5), source and docs | https://docs.rs/rmcp | The MCP server: tool listing, tool calls, the stdio transport |

## Open standards

None used yet.

## Libraries

Open source libraries are dependencies, listed in `Cargo.toml` and
`app/package.json`. Their licences are checked by `cargo deny` on every merge.
