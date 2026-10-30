# omron-mcp

A single-binary MCP server that lets the Claude apps (iPhone, web, Desktop, Claude Code)
and ChatGPT read your OMRON blood pressure readings: ask for last week's average, the
trend over a month, or the readings flagged with an irregular heartbeat.

It signs in to OMRON connect with your account, keeps a small cache of your readings, and
answers over MCP Streamable HTTP at `<public_url>/mcp`. Claude is authorized through a small
OAuth 2.1 server for one owner. The server, OAuth and state-file blocks and the CLI plumbing
come from [mcp-starter](https://github.com/risan/mcp-starter); the OMRON client is the
`omron-connect` crate in this repository. It is written in Rust and sized for a tiny Alpine
LXC: a static musl binary, one thread, and no work while idle (see "Resource use").

Setup, from the Alpine LXC to a test prompt: see the [setup guide](docs/setup.md).
The server also serves this README as a public page at `/`.

## Tool

| Tool | What it does |
|---|---|
| `omron_get_blood_pressure` | Syncs with OMRON connect, then returns the readings of a date range (default the last 30 days, at most 366) newest first, with the count and the average, min and max of systolic, diastolic and pulse |

Dates are `YYYY-MM-DD`, taken in each reading's own time zone, and the default end date is
today in `OMRON_MCP_TZ`. `user` picks one user slot of a two-user cuff. Each reading has its
local `date`, `time` (with seconds), `utc_offset`, `systolic` and `diastolic` (mmHg),
`pulse` (bpm), `irregular_heartbeat`, `body_movement`, `cuff_wrap_ok`, `device` (the model)
and `user`. The device serial number is never shown.

When OMRON cannot be reached, or the login has expired, the tool still answers from the
cache and says so in a `note`. A first sync of a long history can take minutes; the tool
then answers with what is cached and the sync carries on in the background.

## Supported OMRON accounts

OMRON runs two cloud systems and picks one by country. The v1 system (Indonesia and most of
Asia, Australia and Latin America) is verified against a live account. The v2 system (Europe,
the Americas and others) is implemented from the protocol notes of the omramin project and
is **untested**: it has never run against OMRON's servers. Countries OMRON serves from no
cloud are refused at login.

## Commands

```
omron-mcp serve           run the server (default)
omron-mcp login           sign in to OMRON connect (country, email, hidden password) and run the first sync
omron-mcp status          login, sync and password state, no secrets
omron-mcp set-password    set the owner password (12+ characters)
omron-mcp revoke-tokens   delete all OAuth tokens and registered clients
```

Settings come from the environment: `OMRON_MCP_PUBLIC_URL` (required for `serve`),
`OMRON_MCP_LISTEN` (`127.0.0.1:8082`), `OMRON_MCP_DATA_DIR` (`/var/lib/omron-mcp`) and
`OMRON_MCP_TZ` (`Asia/Jakarta`).

The password is used once, at `login`, and never stored. The data directory holds
`state.json` (the OMRON tokens and the OAuth state) and `readings.json` (the cache), both
mode 0600. The server refreshes the OMRON token every 24 hours so an unused login stays
alive.

## Resource use

Resource use is the main design goal: the box this runs on hosts other services too. The
release check (`tests/loadcheck.rs`) starts the binary against a fake OMRON, with a state
file of 50 clients and a year of rotated refresh tokens and a cache of 2,000 readings, and
reads `/proc`. Each of its 10 burst rounds runs 9 tool calls (a one-year range, new readings
to sync) and 1 token rotation. Three runs on x86-64, musl build:

| | Limit | Measured (3 runs) |
|---|---|---|
| Binary | static musl, at most 8 MB | 4.37 MB (4,372,776 bytes; production) |
| Idle RSS (70 s after the last OMRON call) | at most 8 MB | 4.15 MB (4,148 kB in all runs) |
| RSS after 90 tool calls and 10 token rotations | at most 8 MB (8,192 kB) | 7.21 to 7.55 MB |
| Peak RSS during that burst | at most 10 MB | 8.56 to 8.70 MB |
| Peak live heap | at most 2,500,000 bytes | 2,080,423 to 2,080,433 bytes |
| CPU time over 60 idle seconds | unchanged | 0 ticks in all runs |
| Threads while idle and after the burst drains | 1 | 1 in all runs (3 during the burst) |

The peak comes from the biggest reply: the burst asks for a year of readings, which is 921
of the 2,000 cached ones, built as JSON values and sent twice (as structured content and as
text). A year of one or two readings a day is half of that. Idle, the process holds none of
the cache.

The binary uses mimalloc as its allocator. The libc malloc of the static build (Zig's)
retains freed small allocations and never unmaps those slabs, so a burst would leave 9 to
14 MB of RSS over a heap under 1 MB (measured in the sibling mfp-mcp project). Transparent
huge pages would add megabytes to the idle RSS: mimalloc is built with `no_thp`, so it never
asks for them, and `main` turns them off for the process (`prctl(PR_SET_THP_DISABLE)`).
`main` also tells mimalloc to purge freed memory at once, because an idle process makes no
allocator calls and a delayed purge would never run. A unit test checks the option's index
still names purge_delay.

A sync reply on a pulse-wave cuff carries about 220 KB of waveform per reading. The client
parses the reply while it downloads and skips the waveform without buffering it, so memory
stays flat however large the first sync is: the `omron-connect` memory test syncs 30 MB with
a heap peak under 4 MB. The readings cache is loaded for one call and dropped again.

Run the check yourself:

```sh
scripts/build-release.sh --bench
OMRON_MCP_BENCH_BIN=$PWD/target/bench/x86_64-unknown-linux-musl/release/omron-mcp \
  cargo test --release --test loadcheck -- --ignored --nocapture
```

## Develop

```sh
cargo test --workspace                                # everything runs against in-process fakes
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
scripts/build-release.sh                              # static musl binary (needs zig, cargo-zigbuild)
```

The live test reads from a real account and never runs by default. It needs your own
credentials in the environment:

```sh
OMRON_EMAIL=... OMRON_PASSWORD=... OMRON_COUNTRY=ID \
  cargo test --features live --test live -- --nocapture
```

## Credits

The protocol, in particular the v1 sync endpoint, was documented by
[omramin](https://github.com/bugficks/omramin) and
[aircon-chen/omron-connect-mcp](https://github.com/aircon-chen/omron-connect-mcp). Both are
GPL licensed, so this is an independent implementation written from those protocol notes,
not from their code, and is released under the MIT license. The HTTP server, OAuth server,
state file and CLI blocks are [mcp-starter](https://github.com/risan/mcp-starter). Not
affiliated with OMRON Healthcare.
