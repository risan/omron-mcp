# Scout: garmin-mcp

Citations are file:line or file:symbol, with paths under /home/risan/projects/code/garmin-mcp unless noted. The scout read the code and docs but ran no build or test.

## Repo rules
- There is no AGENTS.md or CLAUDE.md in the repo. The only applicable rule is /home/risan/projects/CLAUDE.md: "Never add a `Co-Authored-By: Claude` trailer or a `Claude-Session:` trailer to commit messages in any repository under this directory." It overrides the attribution reminder in this session. .works/blood-pressure/build-report.md ("No Co-Authored-By or Claude-Session trailers in commit messages (repo rule)") shows it was followed.
- Fixture leak scan (memory fixture-leak-scan.md, from MEMORY.md): builders copy real probe values into fixtures, so scan the diff before every merge. The blood-pressure build report did this ("Leak check: all fixture values ... are invented").
- tools/list budget: `tests/tools_list_snapshot.rs:96-97` says "Shorten descriptions before raising it" about `MAX_TOOLS_LIST_BYTES`.

## Map
- `src/main.rs` — mimalloc global allocator, `configure_allocator()`, then `mcp_starter::runtime::run_main(... cli::run ...)`.
- `src/lib.rs` — `SERVICE` ServiceInfo (env_prefix, default listen, data dir, tz, scope).
- `src/cli.rs` — commands, `dispatch`, `serve()` (the only place a background task is spawned), `ToolOptions` and testhooks.
- `src/state.rs` — `GarminState`, the service's one state-file section.
- `src/garmin/mod.rs` — `Client`, `Error::RateLimited`, retry/429 logic, `run_keep_alive`.
- `src/garmin/api/body.rs` — weight and blood pressure API structs and calls.
- `src/tools/body.rs` — the blood pressure tools (get, log, delete).
- `src/tools/mod.rs` — `App`, `GarminTools::router()` (tool registration), `run`/`run_write`.
- `tests/tools_list_snapshot.rs` and `tests/snapshots/tools_list.json` — snapshot and size guard.
- `tests/tools.rs` — hard-coded READ_TOOLS and WRITE_TOOLS lists (`:57`, `:90`) and a count check (`:244`).
- `tests/tools_blood_pressure.rs` — 13 blood pressure tool tests.
- `tests/deploy_kit.rs` — pins the deploy files.
- `deploy/alpine/garmin-mcp.initd` and `deploy/alpine/garmin-mcp.confd` — the OpenRC deploy kit.
- `docs/setup.md` — the "second service on the LXC" recipe.
- `.works/blood-pressure/{brief,build-report}.md` and `.works/rust-port/deploy-report.md`.

## Current behavior

### 1. Blood pressure

**Log**
- The call is `POST bloodpressure-service/bloodpressure` — `src/garmin/api/body.rs`, `log_blood_pressure`. A write is sent once, with no retry — `src/garmin/mod.rs:1064-1065`, `write`.
- Body fields (`src/tools/body.rs:171-178`):
  - `measurementTimestampLocal`
  - `measurementTimestampGMT`
  - `systolic`
  - `diastolic`
  - `sourceType: "MANUAL"` (hard-coded)
  - `notes` (empty string if none)
  - `pulse`, only when given
- Timestamps use the format `%Y-%m-%dT%H:%M:%S.000` — `src/tools/body.rs:550`, `garmin_timestamp`. The local one is the moment in the configured zone (`config.tz`, default `Asia/Jakarta`). The GMT one is the same instant converted to UTC (`:172-173`). Seconds are always 00 (`logged_moment` builds the time as hour and minute only, `:444`, or from an HH:MM string).
- `logged_moment` (`:427-452`) refuses future dates and times. The date defaults to today. With no time it uses now for today and 08:00 (`DEFAULT_LOG_TIME`) for earlier dates.
- Validation (`check_pressures`, `:464`): systolic and diastolic must lie in the `SYSTOLIC_MMHG` and `DIASTOLIC_MMHG` ranges (70–260 and 40–150 per the input docs), pulse 20–250, and systolic must be greater than diastolic.
- The POST reply has no `version`, so no id comes back. Live verification: `.works/blood-pressure/build-report.md` "Live verification"; struct doc at `src/garmin/api/body.rs`, `LoggedBloodPressure` ("Live on 2026-10-30 it had no `version`").
- The tool therefore returns no `reading_id` unless Garmin starts sending one (`src/tools/body.rs:196`).
- Updating a saved reading is not possible: delete and re-log (`DESCRIPTION_LOG_BLOOD_PRESSURE`, `:42`).

**Read**
- The call is `GET bloodpressure-service/bloodpressure/range/{start}/{end}?includeAll=true` — `src/garmin/api/body.rs`, `blood_pressure_range`.
- The reply is `measurementSummaries[]` of `{startDate, measurements[]}`. Each `BloodPressureReading` has `version`, `systolic`, `diastolic`, `pulse`, `measurementTimestampLocal`, `measurementTimestampGMT` (Strings), `sourceType` and `notes` (`src/garmin/api/body.rs`, `BloodPressureReading`).
- The memory file garmin-read-api-facts.md line 19 adds `multiMeasurement`, `category` and `categoryName`. The code does not read these because Garmin's two category scales disagree.
- `version` is the reading id, and it is the key for delete. Tool range: default 30 days, maximum 366 (`src/tools/body.rs:26-27`).
- `reading_output` (`:614-649`) emits `reading_id`, `date` (the day summary's `startDate`, else the date of the local timestamp), `time_local` (first 16 characters of the local string, so minute precision), `systolic`, `diastolic`, `pulse` (dropped if ≤ 0), `notes` (dropped if blank) and `source`. It drops a reading without both pressures, and sorts newest first by the GMT timestamp via `gmt_text_time` (`src/tools/output.rs:10`).
- Fields that could identify a reading for dedupe:
  - `version` (the Garmin id), usable on read only.
  - The GMT timestamp plus systolic and diastolic; the live test finds its reading "by values + time + notes" (build-report S5).
  - `notes`, which round-trips on read.
  - `sourceType` is exposed as `source`; the owner's own readings are `MANUAL`.

**Delete**
- `DELETE bloodpressure-service/bloodpressure/{date}/{version}` — `src/garmin/api/body.rs`, `delete_blood_pressure`. The date is the YYYY-MM-DD the reading is listed under. Verified live (build-report).

### 2. Config
- Config comes from mcp-starter 0.1.5 (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/mcp-starter-0.1.5/src/config.rs:37-65`). It reads only `<env_prefix>PUBLIC_URL`, `LISTEN`, `DATA_DIR` and `TZ` through a `getenv: &dyn Fn(&str)->String`. For garmin-mcp that is `GARMIN_MCP_*` (`src/lib.rs`, `SERVICE`).
- `Config` has no optional or extension section. An unset var returns an empty string, so "unset" means empty.
- `getenv` is passed into `cli::run` (`src/cli.rs:83`) and `dispatch`. `dispatch` loads config with `Config::load(&SERVICE, &*getenv, command == Command::Serve)` (`:114`), and `tool_options(&getenv)` builds `ToolOptions` (`:119`, `:140-151`). That is the existing path for reading extra env vars.
- Nothing in garmin-mcp reads any env var other than testhooks (`GARMIN_MCP_TEST_*`, compiled only with feature `testhooks`).
- Deploy env vars live in `deploy/alpine/garmin-mcp.confd` (exported; OpenRC sources it).

### 3. State store
- File: `<DATA_DIR>/state.json` (default `/var/lib/garmin-mcp`, `docs/setup.md`: "`/var/lib/garmin-mcp/state.json` will hold your Garmin login and the OAuth token hashes"). Directory is garmin:garmin 0700.
- The file has the top-level keys `garmin`, `owner`, `clients` and `tokens`, plus an `extra` map (mcp-starter `src/store.rs`, `State`). Unknown top-level keys are kept on every save (the module docs say "Unknown keys survive every save").
- Garmin tokens live under key `garmin` (`src/state.rs`, `impl Section for GarminState`, `KEY = "garmin"`). Fields: `login_rev`, `access_token`, `refresh_token`, `client_id`, `display_name`, `full_name`, `access_expires_at`, `refresh_expires_at`, `last_refresh_at`, `login_invalid`. Empty strings are skipped on write.
- `Store<GarminState>` is generic over exactly one `Section` type (`Section` trait, store.rs:29). The section key must not be `owner`, `clients` or `tokens`.
- `store.read(|state| ...)` and `store.update(...)` are the access methods (store.rs:390, :409). Atomic cross-section updates are possible only inside the one `State<GarminState>`.
- A second upstream's tokens and a sync cursor would therefore need either new fields on `GarminState` (the serde `#[serde(default)]` struct makes this backward compatible) or a different section type. The `extra` map is preserved but is not typed.
- `GarminState`'s `Debug` impl hides the token values (`src/state.rs`); a test pins it (`debug_hides_the_tokens`). The test `empty_section_json` pins the exact empty JSON `{"garmin":{"login_rev":0},"owner":{},"clients":{},"tokens":{}}`, so a new always-serialized field would change it. Existing fields use `skip_serializing_if`.
- Only one process holds the state file lease (`Serving::start` takes the lease); CLI commands like `login` run beside `serve` only via the lease/store design.

### 4. Background work and startup
- `main` runs `configure_allocator()` then `run_main` (`src/main.rs:49-55`). `cli::serve` (`src/cli.rs:219-250`) calls `Serving::start(&SERVICE, config, store, shutdown)`, builds the garmin client, and spawns the keep-alive on `serving.work.tracker()`. It then builds `App`, then `serving.run_with_routes(io, GarminTools::factory(app), upload::routes(app))`.
- This is the only background task: `run_keep_alive` (`src/garmin/mod.rs:670-681`) runs `keep_alive_once` at start and then every `KEEP_ALIVE_EVERY` = 24 h (`:41`) in a `tokio::select!` against the shutdown token. Errors go to `eprintln!("garmin keep-alive: ...")`.
- `tokio::spawn` is not used. `Work::spawn` (mcp-starter `src/work.rs:104`) is used for tool calls and uploads (`src/upload.rs:191`). It refuses new tasks once shutdown has begun.
- Shutdown is via `serving.shutdown` (a CancellationToken); the process stops on SIGINT or SIGTERM unless `Settings.shutdown` is given.
- tokio features are `rt, net, time, signal, sync, macros` (single thread; README: "one thread, and no work while idle"). The README and deploy report stress an idle server doing no work; a periodic timer adds a wake-up. The Cargo.toml tokio feature list has no `rt-multi-thread`.

### 5. Tool registration
- `GarminTools::router()` (`src/tools/mod.rs:147-164`) adds seven macro-generated routers (`daily_router + training_router + activities_router + workouts_router + courses_router + races_router + body_router`), then strips `$schema` and `format` from each input schema.
- `factory(app)` builds the router once and shares it as `Arc<ToolRouter<GarminTools>>` (`:135-144`). `#[tool_handler(router = self.router, ...)]` is on `ServerHandler` (`:309`). `call_tool` is overridden to wrap `mcp_starter::tools::call_tool` (`:310-323`).
- Conditional listing: nothing today is conditional. All 52 tools are always listed. The router is a plain `ToolRouter` with a pub `map` (the code already does `router.map.values_mut()`), and rmcp 3.7.0 has `ToolRouter::remove_route(&mut self, name)` (rmcp `src/handler/server/router/tool.rs:456`). `App` does not carry an Omron-configured flag today (fields: garmin, config, tools, work, uploads).
- Size guard: `tests/tools_list_snapshot.rs:97` `MAX_TOOLS_LIST_BYTES: usize = 56_000`. The test `tools_list_stays_within_its_size_budget` (`:99+`) serialises `env.mcp.list_tools()` from `ToolsEnv::start()` and compares. Headroom: the trim report records 55,964 bytes, so about 36 bytes are left under 56,000 (`.works/trim-tool-list/build-report.md`; `.works/rust-port/deploy-report.md` last update says "55,964 ... guards it at 56,000"). A new tool listed in that test environment would exceed the budget unless the test env leaves it hidden or the budget is raised (the file comment says shorten descriptions first).
- Snapshot: `tests/snapshots/tools_list.json`, sorted by name; regenerate with `UPDATE_TOOLS_LIST=1 cargo test --test tools_list_snapshot` (`tests/tools_list_snapshot.rs:3`). `tests/tools.rs` keeps hard-coded lists (READ_TOOLS: 29, WRITE_TOOLS: 23, count check at `:244`). The blood-pressure build report notes these were missed once.
- Tool shape to mirror: `#[tool(name=..., description=CONST, annotations(...))]` inside `#[tool_router(router = body_router, vis = "pub(super)")]` (`src/tools/body.rs:98-ish`). A write uses `self.run_write(|garmin| async move {...})` (`src/tools/mod.rs:264`) and a read uses `self.run(...)`. A tool is bounded by `tool_deadline` of 40 s (`ToolOptions::default`), and a write that times out says the change may have landed (`MAYBE_SAVED`, `src/garmin/mod.rs:50`).
- Inputs are `#[derive(Deserialize, JsonSchema)]` structs with doc comments as field docs (`src/tools/body.rs:66-95`).

### 6. Cargo, deps, allocator
- Package `garmin-mcp` 0.1.0, edition 2024, rust-version 1.99, `publish = false`, lib `garmin_mcp` and bin `garmin-mcp` (Cargo.toml).
- Features: `testhooks = []` ("Env-driven hooks the subprocess and load tests need; never in the production build") and `live = []` (manual live test). There are no other features.
- Dependencies are pinned by version requirement from crates.io, not git: `mcp-starter = "0.1.5"` (dev-deps adds `features = ["test-support"]`), `rmcp = "3.7.0"`, `reqwest = "0.13.5"` (default-features false, `rustls-no-provider`, json, query), `tokio = "1.54.1"`, `jiff = "0.2.38"` (`tzdb-bundle-always`), `axum`, `serde`, `serde_json`, `schemars`. There is no git or path dependency. Memory says mcp-starter is a published crate. Cargo.lock is committed.
- A path or git dependency on a new `omron-connect` crate would be the first non-registry dependency. Whether it is published to crates.io is not settled here (see Open decisions).
- Release profile: `opt-level = "s"`, fat LTO, `codegen-units = 1`, `panic = "abort"`, stripped (Cargo.toml `[profile.release]`).
- Allocator: `mimalloc = { version = "0.1.52", features = ["no_thp"] }` as `#[global_allocator]` (`src/main.rs:5-6`). `configure_allocator` sets `mi_option_purge_delay` to 0 (`MI_OPTION_PURGE_DELAY = 15`, index pinned by a test) and `prctl(PR_SET_THP_DISABLE)`. `src/lib.rs` has `#![forbid(unsafe_code)]` while `main.rs` holds the unsafe FFI.
- Build: `scripts/build-release.sh` runs `cargo zigbuild --release --target x86_64-unknown-linux-musl` (aarch64 via `TARGET=`). `--bench` adds `--features testhooks` in `target/bench`.
- The static musl binary is 6,216,104 bytes (deploy report); the README target is at most 8 MiB. Any added HTTP/TLS or crypto dependency for Omron grows it.

### 7. 429 / rate limits
- Reads: a 429 is an error without a retry; a 5xx or a transport failure is tried once more after `retry_delay` (1 s default) unless the deadline is near (`src/garmin/mod.rs:1028-1056`, `:124-138`).
- Writes: sent exactly once; 429 maps to `Error::RateLimited` and a network failure appends `MAYBE_SAVED` (`:1064-1090`).
- `Error::RateLimited` displays "Garmin rate limit, try again later" (`:86`). Sign-in treats 429 the same (`:939`). The login-id loop returns at once on a rate limit (`:383`).
- There is no backoff, queue or Retry-After parsing anywhere. The only periodic Garmin caller is the 24 h keep-alive.
- Verified live facts (memory garmin-write-api-facts.md line 21): "Create, list, then delete back to back → 429. Pause between them" (courses). Blood pressure log and delete in sequence worked in the live write test, but no burst limit for blood pressure is recorded.
- A refresh failure is remembered (`RefreshSlot.failed_at`, `EARLY_RETRY_AFTER` 60 s, `:39`), which stops refresh stampedes but does not govern writes.

### 8. Deploy
- Host: CT 103 `public-apps` (Alpine LXC, 192 MB, 1 core, 2 GB disk; `.works/rust-port/deploy-report.md`, Done section).
- Service manager: OpenRC with `supervise-daemon` (`deploy/alpine/garmin-mcp.initd`: `supervisor="supervise-daemon"`, `respawn_delay=2`, `respawn_max=5`, `retry="TERM/180/KILL/5"`, `command_user="garmin:garmin"`, `command_args="serve"`, `depend() { need net }`).
- Binary: `/usr/local/bin/garmin-mcp`, with the previous one kept as `/usr/local/bin/garmin-mcp.prev`.
- Init script: `/etc/init.d/garmin-mcp`. Config: `/etc/conf.d/garmin-mcp`. Logs: `/var/log/garmin-mcp/garmin-mcp.log` and `.err`.
- State: `/var/lib/garmin-mcp/state.json`, dir 0700 garmin:garmin; the user is `garmin` (uid 102).
- Port and domain: `127.0.0.1:8081`, public `https://garmin.risanb.com` (conf.d exports `GARMIN_MCP_PUBLIC_URL`, `GARMIN_MCP_LISTEN`, `GARMIN_MCP_DATA_DIR`, `GARMIN_MCP_TZ=Asia/Jakarta`).
- Memory cgroup: `memory.high 24M`, `memory.max 32M` (`rc_cgroup_settings`). RSS is about 5 MiB at start (deploy report).
- Neighbours in the same LXC: `mfp-mcp` (own service, memory cap 32 MB per docs/setup.md:22) and `cloudflared` (capped at 64 MB per docs/setup.md:22). One shared tunnel `public-apps`; a second hostname is a published-application route to its loopback port, with no second cloudflared (`docs/setup.md` section 7).
- A second service follows `docs/setup.md` sections 2–7: its own user and group, data dir 0700, initd and confd, a conf.d memory cap, a different loopback port, a new tunnel route. `mfp-mcp` and garmin-mcp are the existing two (mfp default port is not stated here; `MFP_MCP_LISTEN` appears only as a decoy in `src/lib.rs` tests). `docs/setup.md` says the per-service caps need the `cgroup-delegate` service installed by the mfp-mcp guide. Total LXC memory plan: Alpine about 10 MB + 32 + 32 + 64 MB cloudflared, within 192 MB (`docs/setup.md:22-25`).
- Deploy practice per update: copy the binary, `rc-service garmin-mcp restart`, check healthz, login valid, public `/` 200, `POST /mcp` 401 (deploy report entries).
- `tests/deploy_kit.rs` pins the initd and confd contents.

## Pattern
- `src/tools/body.rs` — mirror the get/log/delete trio: input struct with doc comments, `DESCRIPTION_*` const, `#[tool(...)]` with annotations, `run_write` for writes — `src/tools/body.rs:41-43,66-95,153-205`.
- `src/garmin/api/body.rs`, `blood_pressure_range` — the typed range read and the `log_blood_pressure(&Value)` call are directly reusable for a sync, since the same `Client` is held in `App.garmin`.
- `src/tools/body.rs`, `logged_moment` and `garmin_timestamp` — how a local wall-clock time and its UTC twin are turned into the two Garmin timestamps.
- `src/cli.rs:230-235` — the pattern for a background loop: `serving.work.tracker().spawn(async move { client.run_keep_alive(shutdown).await })`, with the loop in `src/garmin/mod.rs:670-681` (check shutdown, do the work, `select!` on shutdown vs sleep). It does not use `tokio::spawn`.
- `src/state.rs`, `GarminState` — how a section is declared (`impl Section`, `KEY`, `#[serde(default)]`, `skip_serializing_if`, a redacting `Debug`).
- `src/cli.rs`, `tool_options(&getenv)` and `ToolOptions` — how a non-config setting reaches the tools (via `App.tools`).
- `src/tools/mod.rs:147-164` — the `router()` assembly, where a route could be added or removed.
- Test patterns: `tests/tools_blood_pressure.rs` (13 tests against a fake Garmin) and `tests/tools_list_snapshot.rs`.

## Tests
- Framework: Rust built-in `cargo test` with tokio tests; fake Garmin servers in `tests/support`. There is also `tests/live.rs` under feature `live`, manual and not run by default.
- Command: `cargo test --no-fail-fast` (from .works/blood-pressure/build-report.md, "553 passed"; later 558 passed per the trim report). `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` were also run (same report). No CI file was found. Snapshot regeneration is `UPDATE_TOOLS_LIST=1 cargo test --test tools_list_snapshot`.
- Closest test: `tests/tools_blood_pressure.rs` — covers get/log/delete against the fake API (13 tests, per the build report). Tool-list and size: `tests/tools_list_snapshot.rs`. Tool name/annotation lists: `tests/tools.rs:57,90,244`. State shape: `src/state.rs` unit tests. Deploy files: `tests/deploy_kit.rs`.

## Contracts
- Garmin BP endpoints above (POST/GET range/DELETE), body keys `measurementTimestampLocal`, `measurementTimestampGMT`, `systolic`, `diastolic`, `pulse`, `sourceType`, `notes`.
- State file: top-level JSON `{garmin, owner, clients, tokens, ...extra}`; the empty form is pinned by `src/state.rs` test `empty_section_json`.
- Env: `GARMIN_MCP_PUBLIC_URL`, `LISTEN`, `DATA_DIR`, `TZ` (mcp-starter `config.rs:37-65`).
- tools/list snapshot `tests/snapshots/tools_list.json` and the 56,000-byte budget.
- Tool annotations (log: not destructive, not idempotent; delete: destructive, idempotent; get: read-only, idempotent) — `src/tools/body.rs:121-123,153-155,208-210`.
- `Section` trait requires `Serialize + DeserializeOwned + Default + Clone + Debug + PartialEq + Send + Sync + 'static` (mcp-starter store.rs:29).

## Must not change
- Tool count 52 and tool list contents when Omron is unset: `tests/tools_list_snapshot.rs` snapshot and `tests/tools.rs:244` count test pin them.
- tools/list under 56,000 bytes (currently 55,964): `tests/tools_list_snapshot.rs:97-109`.
- Empty state JSON shape: `src/state.rs`, `empty_section_json`.
- Existing log/delete BP request shape (verified live 2026-10-30): `tests/tools_blood_pressure.rs`.
- Idle behavior ("no work while idle", one thread, RSS about 5 MiB under a 32 MB cap): README line 14; deploy report. A timer must stay off when Omron is unset.
- Allocator setup in `src/main.rs` (pinned by `the_purge_delay_index_names_purge_delay`).
- Deploy files: `tests/deploy_kit.rs` asserts exact lines in initd/confd.
- `#![forbid(unsafe_code)]` in `src/lib.rs`.

## Open decisions
- How the optional Omron section is read: mcp-starter `Config` has no extra-env hook, so it would go through `getenv` in `cli.rs` like `tool_options`. Whether credentials go in env vars, `state.json`, or an interactive CLI command is not settled by the repo; the existing pattern is `login`/`import-tokens`/`set-password` commands storing secrets in `state.json`.
- Where Omron tokens and the sync cursor live: new fields on `GarminState`, a different `Section` type, or a separate file. The repo supports only one typed section.
- How "hidden when unset" is done: the router is built once with all tools, so it needs a conditional build or `remove_route`, plus a snapshot and size-guard plan (the snapshot test env defines whether the tool counts toward 56,000 bytes). The repo does not decide this.
- How `omron-connect` is depended on (path, git, or crates.io). Today everything is crates.io, and `publish = false` for garmin-mcp itself.
- Dedupe key: the POST reply gives no id, and `version` appears only on read. The repo does not define the match rule (values + GMT minute + notes vs. a marker in `notes`). It does not say whether Garmin keeps seconds or truncates them, and `time_local` is truncated to minutes on output.
- Source type: the log call hard-codes `MANUAL` (`src/tools/body.rs:176`), and the owner logs readings by hand as `MANUAL` (memory). Whether a synced reading should be distinguishable (for example by a `notes` marker) is open.
- Time zone of Omron timestamps vs. `config.tz` (default `Asia/Jakarta`): not determined here.
- Write pacing for a periodic writer: no BP rate-limit data beyond the course 429; no backoff exists.
- How the timer interacts with shutdown (the tracked keep-alive pattern exists) and how a sync would use `until(deadline)` clients: `run_with_note` shows the per-call deadline mechanism, but no non-tool caller of `App.garmin` exists besides the keep-alive.
- The mfp-mcp port and its init details: not in this repo.
