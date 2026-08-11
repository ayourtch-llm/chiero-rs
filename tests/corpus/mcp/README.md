# The MCP schema, vendored

`schema-2025-06-18.json` is the Model Context Protocol's own JSON Schema, fetched 2026-08-11
from

    https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/2025-06-18/schema.json

**HTTP 200, 108 234 bytes, 91 definitions.** Not modified.

## Why it is here rather than fetched

A test that needs the network is a test that fails on a train — and worse, one that *passes*
when the network hands it something else. The whole point of this file is to be an oracle for
`chiero serve`, and an oracle that can change under you is not one.

It also settles an argument this project had with itself. On 2026-08-11 the MCP half of
[`HANDOFF.md`](../../../HANDOFF.md) §9.1 item 6a was recorded as blocked, with the reason *"no
`node`/`npx`, no MCP client and no copy of the protocol schema anywhere on disk — every detail
would be written from memory and tested against nothing"*. That was the fifth false
impossibility of the day: the machine had no schema **on disk** and a working network, and one
`curl` produced this file. §8.3's step 0b came out of the pattern. Vendoring it means nobody has
to rediscover that.

## What it is for

`chiero serve` speaks JSON-RPC 2.0 today and **not** MCP: no `initialize` lifecycle, no content
blocks, no notifications. When that changes, this is what the responses get validated against —
`InitializeResult` requires `capabilities`, `protocolVersion` and `serverInfo`; `CallToolResult`
requires `content`; `Tool` requires `name` and `inputSchema`. `python3 -c "import jsonschema"`
answers 4.10.3 on this machine, so the validation needs nothing else installed.

⚠️ The version is in the filename because MCP versions its schema by date. A newer one is a
*different* oracle, not an upgrade of this one: adding it should mean adding a file, and saying
which version the server claims.

## Licence

The MCP project is mid-transition from MIT to Apache-2.0 — its `LICENSE` says new contributions
are Apache-2.0 and un-relicensed MIT contributions stay MIT. Both are compatible with this
repository's `MIT OR Apache-2.0`. Nothing here is redistributed as chiero's own work; it is a
third-party specification artefact kept for testing, unmodified, with its origin recorded above.
