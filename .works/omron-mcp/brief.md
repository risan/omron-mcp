# Omron blood pressure: omron-mcp server and Omron → Garmin sync

Status: ready
Type: feature
Route: initiative — two repos and four workstreams (Omron client library for two API systems, standalone server, garmin-mcp sync tool and timer, deploy); cannot be listed as one change without decomposing
Next: decompose

## Request
"Checkout this repo https://github.com/aircon-chen/omron-connect-mcp ... My new blood pressure is connected to this Omron Connect app on iOS. It would be great to automatically have access to this as well, so I don't need to log my blood pressure to Garmin manually. It needs to be rust port as well like our garmin-mcp, so it can be deployed within our LXC as well."
Follow-ups: "a separate omron MCP would be useful for other people that don't use garmin mcp", then "Can I have both? Separate omron-mcp and the optional automatic sync tool for omron -> garmin connect?"

## Goal
Blood pressure readings taken on the owner's Omron cuff reach Claude and Garmin Connect with no manual logging. Anyone with an OMRON connect account can run omron-mcp on its own; garmin-mcp users can opt in to an automatic Omron → Garmin sync.

## Target
- `~/projects/code/omron-mcp` (new, empty git repo) — Cargo workspace: `omron-connect` library crate and `omron-mcp` server binary.
- `~/projects/code/garmin-mcp` — optional sync tool and timer (seams in `scout-garmin-mcp.md`).
- Pattern to follow: `~/projects/code/myfitnesspal-mcp/mfp-mcp-rs` (`scout-mfp-mcp-rs.md`) and garmin-mcp, both on `mcp-starter` (garmin is on 0.1.5).
- API facts: memory `omron-probe-facts.md`; probe scripts in the session scratchpad; raw sample `~/.omron-probe/sync-raw.json` (real personal data).

## Acceptance criteria
- [ ] `omron-connect` signs in and refreshes tokens on Omron's older (Kii) API system, verified live on the owner's Indonesia account (`data-sg`).
- [ ] `omron-connect` also supports the newer EU/North America API system, built from public protocol notes; it is marked untested until a live account confirms it.
- [ ] Sync is incremental: it keeps the `lastSyncDate` cursor and a small local copy of past readings, skips the ~220 KB pulse-wave (`ppwData`) records without keeping them in memory, and never refetches full history in normal use.
- [ ] `omron-mcp` exposes blood pressure readings (systolic, diastolic, pulse, quality flags, time with timezone) through MCP tools, behind the same owner OAuth as the other servers.
- [ ] `omron-mcp` meets the mfp-mcp-rs resource gates or better (binary ≤ 8 MB, idle RSS ≤ 8 MB, one thread, no work while idle), measured and recorded in its README.
- [ ] `omron-mcp` is public on GitHub under MIT, README rendered as its home page, "not affiliated with OMRON" stated.
- [ ] `omron-mcp` runs on CT 103 `public-apps` beside garmin-mcp and mfp-mcp, with its own user, data dir, port, memory cap and tunnel hostname, within the LXC's 192 MB.
- [ ] garmin-mcp gains an optional `garmin_sync_omron_blood_pressure` tool that copies new Omron readings into Garmin and skips readings Garmin already has; when Omron is not configured, the tool list, tool count, state JSON and idle behavior are unchanged.
- [ ] garmin-mcp gains an optional timer that runs the same sync on an interval; off unless configured.
- [ ] No real probe values (tokens, serials, readings) appear in any committed fixture.

## Out of scope
- Weight and body-composition data (no Omron scale; upstream parsing is untested).
- ECG and pulse-wave (`ppwData`) data.
- Writing to Omron.
- Syncing Garmin → Omron.

## Assumptions
- Clean-room rewrite: the Python omron-connect-mcp and omramin are GPL, so the Rust code is written from verified protocol facts, not translated — owner chose MIT.
- Omron app ID/key are embedded as public constants, as omramin already publishes them.
- `omron-mcp` deploys as its own service in CT 103, following `docs/setup.md` "second service" recipe — owner chose "same LXC as garmin-mcp".
- The garmin-mcp sync uses the `omron-connect` library directly, with its own Omron login (verified: two logins coexist and refresh tokens are not invalidated by each other).

## Questions
- Q: Which Omron API systems at first? — A: Both (older Kii system tested live; newer EU/NA system untested).
- Q: License and visibility? — A: Public, MIT.
- Q: Where does omron-mcp run? — A: Same LXC as garmin-mcp (CT 103).
