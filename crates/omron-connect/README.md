# omron-connect

A Rust client for the OMRON connect cloud: sign in, keep the token fresh, and read blood
pressure readings incrementally.

- **v1** (Asia, Oceania, Japan, India and others): verified against a live account.
- **v2** (Europe, North America): **untested**. It follows the published notes of the
  [omramin](https://github.com/bugficks/omramin) project and has never run against OMRON's
  servers.

Some cuffs upload about 220 KB of pulse-wave data with every reading. The reply is parsed
as a stream and that data is skipped, so memory stays flat however long the history is.

The crate installs no TLS provider: build the `reqwest::Client` you pass in with one.
Every call that needs a login takes `&mut Session` and may refresh it: save the session
after each call.

Used by [omron-mcp](https://github.com/risan/omron-mcp), an MCP server for the Claude apps.

Not affiliated with OMRON Healthcare. MIT licensed.
