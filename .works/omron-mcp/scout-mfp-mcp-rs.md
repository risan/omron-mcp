# Scout: mfp-mcp-rs

Repo root: /home/risan/projects/code/myfitnesspal-mcp/mfp-mcp-rs. Paths below are relative to it.

## Repo rules
- No AGENTS.md or CLAUDE.md in the repo. The only applicable rules are the global ones: braces on every conditional, and no `Co-Authored-By: Claude` or `Claude-Session:` trailer under /home/risan/projects (/home/risan/projects/CLAUDE.md).
- `rust-toolchain.toml` pins channel "1.99.1" with clippy and rustfmt, target x86_64-unknown-linux-musl, profile minimal. `Cargo.toml:5` has `rust-version = "1.99"` and edition 2024.
- The README "Develop" section gives the repo's check commands: `cargo test`, `cargo test --features testhooks --test shutdown_process`, `cargo clippy --all-targets -- -D warnings` (README.md:134-141).
- The remote is `git@github.com:risan/myfitnesspal-mcp.git`. mfp-mcp-rs is a subdirectory of that repo, and `.works/` is committed.

## Map
- `Cargo.toml` — single package `mfp-mcp` with a lib (`mfp_mcp`) and a bin (`mfp-mcp`). Dependencies and profile are here.
- `src/main.rs` — allocator, THP and purge setup, then `mcp_starter::runtime::run_main`.
- `src/lib.rs` — the `SERVICE: ServiceInfo` static and `HOME_PAGE`.
- `src/cli.rs` — `Settings`, `run`, `dispatch`, and the `serve`, `set-session` and `status` commands.
- `src/app.rs` — the `App` state struct that handlers share.
- `src/store.rs` — `MfpState`, the Section stored under key "mfp".
- `src/tools.rs` (2535 lines) — all 23 tools in one rmcp `#[tool_router]` impl.
- `src/mfp/` — the upstream API client.
  - `mod.rs` (1115 lines) is the `Client`: session, keep-alive, calls.
  - `api.rs`, `cookies.rs`, `diary.rs`, `exercise.rs`, `fasting.rs`, `measurements.rs`, `nutrients.rs`, `water.rs` and `weight.rs` hold per-area wire types and calls.
- `src/alert.rs` — optional ntfy-style alerts. It builds a throwaway reqwest client.
- `build.rs` and `build/render.rs` — render README.md into `assets/home.html`, giving the public home page (`OUT_DIR/home.html`).
- `tests/` — see Tests.
- `scripts/build-release.sh` — the musl build.
- `deploy/alpine/` — the OpenRC files.
- `docs/setup.md` — the Proxmox, Cloudflare and Claude setup guide.
- `.works/rss-burst/report.md` — the allocator root-cause report.

## Current behavior
1. How mcp-starter is used
   - The pin is `mcp-starter = "0.1.3"` in `Cargo.toml:21`. Cargo.lock resolves to 0.1.3 (Cargo.lock:924-925). The dev-dependency adds `features = ["test-support"]` (Cargo.toml:45). Versions 0.1.4 and 0.1.5 are in the local cargo registry, but the repo is on 0.1.3.
   - Config: `Config::load(&SERVICE, &*getenv, command == "serve")` (src/cli.rs:~95).
   - State store: `Store::open(&config.data_dir, store_options(&getenv))` with `Store<MfpState>`. `store.update(|state| ... Change::Write(()))` writes and `store.read(...)` reads.
   - OAuth owner server and routes: `Serving::start(&SERVICE, config, store, shutdown)` and then `serving.run(io, MfpTools::factory(app))` (src/cli.rs serve fn). The shared CLI helpers `shared::set_password`, `shared::revoke_tokens`, `shared::oauth_status` and `shared::read_secret` come from `mcp_starter::cli`.
   - home_page and favicon: they are fields of `ServiceInfo` in src/lib.rs:12-22. That struct has `command`, `display_name`, `env_prefix` ("MFP_MCP_"), `default_listen` ("127.0.0.1:8080"), `default_data_dir`, `default_tz`, `scope` ("mfp"), `favicon` and `home_page`.
   - Routes are built inside mcp-starter (`build_router` in mcp-starter-0.1.3/src/server.rs). They are `/healthz`, the favicon, the home page, the OAuth routes, the protected-resource metadata, and `/mcp`.
   - Other mcp-starter pieces in use: `mcp_starter::tools::{ToolOutcome, call_tool, date_or_today, error_result, request_permit, run_tool}`, `work::{Work, ConnPermit}`, `Clock`/`system_clock`, and `http::{describe, install_crypto_provider}` (src/alert.rs:9). `install_crypto_provider` installs the ring provider (mcp-starter http.rs:9-12).
   - main.rs is: `configure_allocator(); mcp_starter::runtime::run_main(|args, io, getenv| cli::run(args, io, getenv, Settings::production()))` (src/main.rs:49-55).
   - `cli::run` takes args, `Io` and `Getenv` explicitly, so tests can drive it in-process (src/cli.rs:1-2).
   - Commands: `serve` (default), `set-session`, `set-password`, `status`, `revoke-tokens`, `test-alert`, plus `help`, `-h` and `--help` (src/cli.rs:21-29, 56-83). An unknown command prints usage and exits 2. A failure prints "mfp-mcp <cmd>: msg" and exits 1.
   - `Settings` holds `www_base`, `api_base`, `shutdown: Option<CancellationToken>` and `extra_root_pem`. `Settings::production()` fills the real URLs, and tests override the endpoints (src/cli.rs:34-52).
2. Layout
   - It is a single crate, not a workspace. The lib exposes `alert`, `app`, `cli`, `mfp`, `store` and `tools` (src/lib.rs:3-8).
   - The upstream client lives under `src/mfp/`, split per feature area. The MCP tools sit in one flat `src/tools.rs` that imports from `crate::mfp::...`. `App` holds `mfp: Client`, `config`, `tools: ToolOptions`, `work: Arc<Work>`.
3. Credentials and state
   - There is one state file, `<data_dir>/state.json`, written as `state.json.tmp-<rand>` and then renamed (mcp-starter store.rs:348, 570). Files are mode 0600 and the data dir is mode 0700 (mcp-starter store.rs:317, 341, 555, 591, 663).
   - Per-service data is a `Section` under one key: `impl Section for MfpState { const KEY: &str = "mfp"; }` (src/store.rs:36-38). The same file also holds the owner password hash and the OAuth clients and tokens (the fixture README confirms that structure).
   - `MfpState` fields: `session_rev`, `cookies` (BTreeMap), `account_name`, `session_expires_at`, `last_keep_alive_at`, `session_invalid`, `token`, `token_expires_at`, `user_id`, and a `#[serde(flatten)] extra: Map` that preserves unknown keys (src/store.rs:12-33). Empty values are skipped on write with `skip_serializing_if`.
   - Login flow: `set-session` reads a hidden stdin value via `shared::read_secret`, parses it with `parse_session_input` (accepts a bare value or a `Cookie:` header), and validates it live. `client.validate_candidate` runs under `SET_SESSION_BUDGET` of 45 s. On success it applies the snapshot to the store with `store.update` and prints "Session saved for <account>" (src/cli.rs:~292-330).
   - Refresh: `client.run_keep_alive(shutdown, alerts)` is spawned on the tracker in `serve` (src/cli.rs:~215). `status` prints no secrets, and the session line tells the owner to run `mfp-mcp set-session` when it is unset or invalid.
   - `session_rev` increments on each set-session so a running server notices new cookies (src/store.rs:12).
   - The testhooks feature supplies `StoreOptions` overrides (src/cli.rs:~125-200).
4. Tools
   - rmcp 3.7.0 with the macros: `#[tool_router] impl MfpTools { #[tool(name = "...", description = CONST, annotations(title=..., read_only_hint=..., idempotent_hint=..., destructive_hint=...)) async fn x(&self, Parameters(input): Parameters<Input>) -> CallToolResult }` (src/tools.rs:446-455, 492-496).
   - Input structs derive `Deserialize` and `JsonSchema` (schemars 1.2.3). Their `///` doc comments become the field descriptions, for example "Diary date YYYY-MM-DD (default today in the owner's timezone)" (src/tools.rs:~340).
   - `MfpTools::factory(app)` builds the `ToolRouter` once, in an Arc, and returns a closure that mcp-starter's `Serving::run` calls per connection (src/tools.rs:~365-376).
   - Every tool body runs through `self.run(tool_name, writes, |app, not_after| async move {...})`, which calls `mcp_starter::tools::run_tool`. That spawns a detached, tracked task. The HTTP handler waits only until `tool_deadline` (40 s default, `ToolOptions` at src/tools.rs:43-68). On timeout, write tools append "; the change may have been saved: check mfp_get_... before trying again" (src/tools.rs:405-425).
   - Description style: plain `const DESCRIPTION_X: &str` sentences (src/tools.rs:70-100), with `concat!` for the long ones. They state units, defaults and ranges. Tool names are prefixed `mfp_`.
   - Size guard: mfp-mcp-rs has none. `tests/tools_list_snapshot.rs` only compares the full `tools/list` result byte for byte against `tests/fixtures/tools-list.json`, and asserts 23 tools. A bytes-limit guard like the one in garmin-mcp would be new.
   - Another test, `home_page_lists_every_tool` in src/lib.rs, checks that every tool in the snapshot appears in the README-derived home page.
5. HTTP, allocator, profile, size and RSS
   - HTTP client: reqwest 0.13.5 with `default-features = false, features = ["rustls-no-provider", "json", "query"]` (Cargo.toml:25). There is no gzip feature. The ring provider is installed via `mcp_starter::http::install_crypto_provider()`. The client is built with `.http1_only()`, `.redirect(Policy::none())`, `.pool_max_idle_per_host(0)` and `.timeout(...)` (src/alert.rs:62-66). The MFP `Client` keeps a `reqwest::Client` in `Inner` (src/mfp/mod.rs:195). Responses are read with `http::read_capped(response, MAX_RESPONSE_BYTES)`.
   - Other dependencies: tokio 1.54.1 with only `rt, net, time, signal, sync, macros` (a single-thread runtime, "one thread"), jiff with `tzdb-bundle-always`, uuid v4, libc (Cargo.toml:17-30).
   - Allocator: `mimalloc = { version = "0.1.52", features = ["no_thp"] }` as the `#[global_allocator]` (src/main.rs:3-8, Cargo.toml:22-23). `configure_allocator()` sets `mi_option_set(MI_OPTION_PURGE_DELAY /*15*/, 0)` and calls `prctl(PR_SET_THP_DISABLE, 1,...)`. A failed prctl is only reported, not fatal. A unit test asserts that option index 15 is purge_delay by checking its default of 1000 (src/main.rs:~160-180). The reason is in the comments at src/main.rs:3-6 and in `.works/rss-burst/report.md`: Zig's libc malloc kept RSS at 9-14 MB over a 0.86 MB heap.
   - Release profile (Cargo.toml:32-38): opt-level "s", lto "fat", codegen-units 1, panic "abort", strip "symbols", debug false.
   - Build: `scripts/build-release.sh` runs `cargo zigbuild --release --target x86_64-unknown-linux-musl` (set `TARGET=aarch64-unknown-linux-musl` for ARM). It then runs `file`, `ls -l` and prints the size. `--bench` adds `--features testhooks --target-dir target/bench`. It needs rustup with the musl target, zig, and `cargo install --locked cargo-zigbuild`.
   - Measured and targeted (README "Resource use" table, README.md:~85-115, and tests/loadcheck.rs):
     - Binary at most 8 MB, measured 4,966,248 bytes.
     - Idle RSS at most 8 MB, measured 4.97 MB.
     - RSS after the 90-call burst and 10 token rotations at most 8,192 kB, measured 6.36-6.54 MB.
     - Peak live heap at most 1,114,112 bytes, measured about 906-917 kB.
     - 1 thread while idle.
     - CPU over 60 idle seconds unchanged.
   - Measurement: `tests/loadcheck.rs` starts the `testhooks` binary against a fake MFP over HTTPS with a seeded state file (50 clients and a year of rotated tokens) and samples `/proc` (the `sample(pid)` fn). The `testhooks` feature swaps in a counting allocator that wraps mimalloc and prints `heap peak_bytes=.. live_bytes=..` to stderr on exit of `serve` (src/main.rs:62-160). The `live` feature gates tests/live.rs.
6. Deploy (details under Contracts). The README is public. The home page is built from README.md and served at `/`.

## Pattern
- `src/lib.rs` `SERVICE: ServiceInfo` — one static drives config, env prefix, defaults, scope, favicon and home page — src/lib.rs:12-22.
- `src/main.rs` — mimalloc with `no_thp`, then `mi_option_set(purge_delay, 0)`, then `prctl(PR_SET_THP_DISABLE)`, then `run_main`. Copy this block whole. Keep the `testhooks` cfg split with its counting allocator if you want loadcheck-style heap gating.
- `src/cli.rs` `run` / `dispatch` / `Settings` — a CLI that takes all of its inputs as arguments, so tests run it in-process. Production endpoints come from `Settings::production()`. Test hooks are `#[cfg(feature = "testhooks")]` only, so the production binary carries neither the code nor the variable names (src/cli.rs:~113-117).
- `src/store.rs` — implement `Section` with a `KEY`, add `#[serde(default)]` and `skip_serializing_if` on every field, and add `#[serde(flatten)] extra` so unknown keys survive a save.
- `src/tools.rs` `run` / `run_with_note` — tool bodies run as detached tracked tasks bounded by a deadline.
- `build.rs` plus `assets/home.html` — README rendered to the public page at build time. src/lib.rs tests enforce a self-contained page: no script tags, no relative links, no `.works/`.
- `src/mfp/mod.rs` — upstream `Client` that takes a `Store<State>` handle, a clock, a `TaskTracker`, a `request_timeout` and a `keep_alive_every`, and runs a keep-alive task cancelled by a `CancellationToken`.
- Not a pattern to copy: tools.rs is one 2.5k-line file. omron-mcp's plan of a library crate plus a server binary is a different layout from this single crate (see Open decisions).

## Tests
- Framework: built-in cargo test with `#[tokio::test]` and in-process fakes.
- Command: `cargo test` (README.md:136). Also `cargo test --features testhooks --test shutdown_process` and `cargo clippy --all-targets -- -D warnings` (README.md:137-138).
- Dev-dependencies: `mcp-starter` with `test-support`, `axum` for the fake upstream server, `tempfile`, and `rustls`, `tokio-rustls` and `getrandom` for the HTTPS fake. `bcrypt`, `sha2` and `pulldown-cmark` are also listed (Cargo.toml:41-52).
- Mock HTTP: `tests/support/fake_mfp.rs` is an in-process axum fake of the upstream hosts. It records every request (`Call` with host, method, path, query, headers, cookies and body) and the number of requests in flight. `tests/support/mod.rs` has `ToolsEnv` (with a 10 s test `tool_deadline`) and `fixture_path`. `tests/support/server_rig.rs` builds the server rig.
- Closest test: `tests/tools_list_snapshot.rs`, for the tools/list contract. `tests/cli.rs` is the closest for the login CLI, and `tests/state_roundtrip.rs` for state-file round-trip.
- Fixtures: anonymisation is by construction. `tests/fixtures/README.md` lists every raw value as a placeholder such as `fixture-access-token-full` or `fixture-user-1`, and every expiry is in 2099. There are no real upstream payloads in it. The fake MFP is "shaped after the live-verified calls in scout-mfp.md" (tests/support/fake_mfp.rs:1-3). The test TLS CA and leaf (`tests/fixtures/tls/`) are throwaway and the CA key was discarded.
- Other tests of note: `tests/loadcheck.rs` (resource gates), `tests/deploy_kit.rs` and `tests/service_config.rs` (deploy files and config), `tests/shutdown_process.rs`, `tests/home_page_render.rs`, and `tests/live.rs` under the `live` feature.

## Contracts
- Deploy files (deploy/alpine/):
  - `mfp-mcp.initd` is an OpenRC script. It runs `/usr/local/bin/mfp-mcp serve` as `mfp:mfp` under `supervisor="supervise-daemon"`, with `respawn_delay=2` and `respawn_max=5`, logs under `/var/log/mfp-mcp/`, `retry="TERM/180/KILL/5"` (a graceful stop of up to 180 s), and `depend() { need net }`. Its start_pre checkpaths the log dir with mode 0750.
  - `mfp-mcp.confd` exports `MFP_MCP_PUBLIC_URL`, `MFP_MCP_LISTEN="127.0.0.1:8080"`, `MFP_MCP_DATA_DIR="/var/lib/mfp-mcp"` (owned by mfp, mode 0700), `MFP_MCP_TZ="Asia/Jakarta"` and optional `MFP_MCP_ALERT_URL`. It also sets `rc_cgroup_settings` of `memory.high 24M` and `memory.max 32M`.
  - `cgroup-delegate.initd` makes the memory caps work inside a Proxmox LXC. `cloudflared.initd` runs `cloudflared tunnel --no-autoupdate run` as user cloudflared, with TUNNEL_TOKEN from `/etc/conf.d/cloudflared`.
  - The reverse proxy is Cloudflare Tunnel, not nginx. Public HTTPS goes through Cloudflare to cloudflared to `http://127.0.0.1:8080`.
- Host and CT names (docs/setup.md):
  - A new Alpine LXC `public-apps` (pct id 210: `pct create 210 ...`) holds internet-facing services plus the one cloudflared. Size is 1 core, 128 MB RAM, 2 GB disk (setup.md:18-31, 49-53).
  - `internal-apps` is the existing CT 102 (`pct set 102 --hostname internal-apps`). It holds services with no inbound access.
  - The service user is made with `adduser -S -D -H -G mfp -h /var/lib/mfp-mcp -s /sbin/nologin mfp` (setup.md:119).
  - The binary is copied with `scp target/x86_64-unknown-linux-musl/release/mfp-mcp ... root@<lxc-ip>:/root/`, or pushed with `pct push 210`.
  - The setup guide's section "Adding another public service to the same LXC and tunnel" (setup.md:337-) covers a second service. The new service gets its own user, data dir and memory cap.
- Env contract: prefix `MFP_MCP_` and the `ServiceInfo` fields (src/lib.rs). The OAuth scope string is "mfp".
- Home page: README.md is rendered into `assets/home.html`. The page title must be "mfp-mcp: MyFitnessPal for Claude and ChatGPT" (src/lib.rs test).
- License and README:
  - There is no LICENSE file in mfp-mcp-rs (`ls LICENSE*` fails). `Cargo.toml` has `publish = false` and no `license` field. The README Credits cite another project's MIT licence, for the Python project it is based on, and say "Not affiliated with MyFitnessPal".
  - README structure: intro, Tools table, domain notes, Commands, Settings env, Resource use table, Develop (test commands and the live test), Credits. It is rendered as the public home page at `/`.
  - Whether the GitHub repo is public: Unknown. The README says the home page is public, and src/lib.rs tests forbid `.works/` and the setup link on it.
  - mcp-starter itself is at https://github.com/risan/mcp-starter and has a LICENSE file in the 0.1.5 crate.

## Must not change
- The `tools/list` snapshot is pinned byte for byte by `tests/tools_list_snapshot.rs`. This applies to mfp-mcp only, and omron-mcp will need its own fixture.
- Production binary carries no test hooks, pinned by the `testhooks` feature gating (src/main.rs, src/cli.rs).
- The mimalloc purge-delay index is pinned by the unit test `the_purge_delay_index_names_purge_delay` (src/main.rs). An mimalloc upgrade can move it.
- State file compatibility with the Go build and the earlier Rust build is pinned by `tests/store_go_compat.rs`, `tests/state_roundtrip.rs` and `tests/fixtures/*.json`. This is not relevant to a new repo.
- Resource gates (binary at most 8 MB, RSS at most 8,192 kB, heap at most 1,114,112 bytes, 1 thread) are pinned by `tests/loadcheck.rs`.

## Open decisions
- Workspace layout. mfp-mcp-rs is a single crate with a lib plus a bin, while the omron-mcp plan has two crates (`omron-connect` library and `omron-mcp` binary). The repo does not say how to split `testhooks`, `build.rs` or the loadcheck across crates.
- mcp-starter version. mfp-mcp-rs is on 0.1.3. 0.1.4 and 0.1.5 exist in the local cargo registry, and I did not diff them. Memory notes say 0.1.3 is the latest known, so the repo does not settle which to use.
- Whether omron-mcp needs a `set-password`/OAuth owner flow. The reference has it via mcp-starter, and the Omron login CLI command is new (credential or token capture is not decided).
- License choice for the new public repo. The reference has no LICENSE file and `publish = false`.
- Whether to add a tools/list size guard. The reference only has the snapshot test. The guard in garmin-mcp (56,000 bytes, per memory notes) was not examined here.
- gzip on reqwest. The reference does not enable it. Whether Omron needs it is unknown.
- Host and CT for omron-mcp. The reference deploys to `public-apps` (CT 210). Whether Omron goes there or to `internal-apps` is not settled.
