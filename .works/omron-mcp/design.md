# omron-mcp design

The contract for building `omron-mcp`. Read `brief.md` for the why, and `scout-mfp-mcp-rs.md` for the pattern repo (mfp-mcp-rs).

Hard rules:
- **Clean room.** omramin and aircon-chen/omron-connect-mcp are GPL. Read them only for protocol facts (URLs, field names, codes). Do not translate their code or copy their structure. This repo is MIT.
- **No real data in the repo.** `~/.omron-probe/` holds the owner's real tokens and readings. You may read `sync-raw.json` to learn its *shape*, but every committed fixture value is invented. Before each commit, grep the diff for the values in `~/.omron-probe/sync-raw.json` (device serial, timestamps, readings).
- **Low RAM is a requirement.** It is measured and recorded, like mfp-mcp-rs.
- Code style: braces on every `if`, blank line after a block, comments only for a non-obvious why. Commit messages have no `Co-Authored-By` or `Claude-Session` trailer.

## Layout

A Cargo workspace at `~/projects/code/omron-mcp`:
- `crates/omron-connect/` — the library. No mcp-starter or rmcp dependency, so others (garmin-mcp) can use it alone. MIT, publishable.
- The root package `omron-mcp` — lib plus bin, built like mfp-mcp-rs: mcp-starter (use the latest published version; garmin-mcp uses 0.1.5), rmcp, reqwest with `rustls-no-provider`, a single-thread tokio, jiff, and mimalloc with `no_thp` plus the purge-delay and THP setup copied from mfp-mcp-rs `src/main.rs`.
- Toolchain, release profile, `scripts/build-release.sh` (cargo zigbuild musl): same as mfp-mcp-rs.

## Omron protocol (verified live on 2026-10-30, except where marked)

### Regions
Omron has two API systems. Pick one by ISO country code.
- **v1 (Kii)**: omramin `data/region_v1.json` lists groups of `countries` with `cloudUrl`, `appID` and `appKey`. Groups without `cloudUrl` are offline (unsupported country). v1 wins when a country is in both files.
- **v2**: omramin `data/region_v2.json` lists groups with `region`, `countries` and `api_configuration.prod_base_url` (plus an optional `prod_secondary_base_url`, tried second). `region: "OFFLINE"` means unsupported.
- Embed both tables as Rust data (country lists, URLs, app IDs and keys are facts). Files are in `/home/risan/.cache/claude-tmp/claude-1000/-home-risan-projects-code-garmin-mcp/9211870a-e345-4264-a794-27cceedf38f1/scratchpad/omron-src/omramin/data/`.
- Indonesia (`ID`) → v1 `https://data-sg.omronconnect.com/api`. Verified.

### v1 (verified)
Headers on every request: `User-Agent: OmronConnect/011.004.00000.001 CFNetwork/1335.0.3.4 Darwin/21.6.0)`, `X-OGSC-SDK-Version: 000.005`, `X-OGSC-App-Version: 011.004.00000`, `X-Kii-AppID`, `X-Kii-AppKey`, `Content-Type: application/json`. Do not send `Accept-Encoding: gzip` (omramin notes the server can mislabel bodies as gzip).
- Login: `POST {server}/oauth2/token` with `{"username": email, "password": password}`. Reply: `access_token`, `refresh_token`, `expires_in` (3600).
- Refresh: same URL with `{"grant_type": "refresh_token", "refresh_token": ...}`. Same reply. Verified: two logins on one account coexist, and an already-used refresh token still works.
- Authenticated calls add `Authorization: Bearer <access_token>`.
- Sync: `POST {server}/apps/{appID}/server-code/versions/current/synchronizeMeasureData` with `{"lastSyncDate": <ms>, "countOnlyFlag": 0}`.
  - Reply: `{"returnedValue": {"lastSyncDate": <ms>, "syncList": [...]}}`. `lastSyncDate` is the cursor for the next call. When nothing is new, the reply is tiny and **has no `lastSyncDate`**: keep the old cursor.
  - `countOnlyFlag: 1` returns only `{"returnedValue": {"lastSyncDate", "countSyncData"}}`. Use it as a cheap "anything new?" check.
  - Shape: `syncList[] → {objectId, transferStartDate, deviceCategoryList[] → {deviceCategory (string), deviceModelList[] → {deviceModel, deviceSerialIDList[] → {deviceSerialID, userNumberInDevice, measureList[] → measure}}}}`.
  - A measure has `timeZone` (IANA name), `deleteFlag`, `transferDate`, `measureDateFrom`, `measureDateTo` (epoch ms; use `measureDateTo` as the reading time), `measureDeviceDateFrom/To` (strings), `measurementMode`, `userUpdateDate`, sometimes `sequenceNumber`, and `bodyIndexList`.
  - `bodyIndexList` maps a string code to `[value, unit, scale, measurementId]`, all strings. The number is `value × 10^scale`. Codes: `1` systolic, `2` diastolic, `3` pulse, `6` irregular heartbeat flag, `7` body movement flag, `8` cuff-wrap flag (it is 1 on all the owner's normal readings, so treat 1 as "cuff wrapped OK"; this meaning is an educated guess, say so in the doc comment). Other codes (10–15, 35, ...) are ignored.
  - **Pulse-wave records.** Every reading on the owner's cuff comes with a second record, `deviceCategory` "61440", whose `bodyIndexList` is `{"ppwData": [36,864 strings]}`, about 220 KB of JSON. Skip any measure whose `bodyIndexList` lacks codes 1 and 2. The parser must skip `ppwData` **without buffering it** (see Memory).
  - `deleteFlag != 0` marks a reading deleted in the app. An incremental sync can deliver this for a reading already cached: the cache must drop it.

### v2 (untested, from omramin)
- Requests with a body carry `Checksum: sha256-hex(body bytes)`, and the body must be compact JSON (no spaces). User-Agent: `OMRON connect/8.2.1 (com.omronhealthcare.omronconnect; build:24; iOS 18.7.2) Alamofire/5.9.1`.
- Login: `POST {base}/login` with `{"emailAddress", "password", "country", "app": "OCM"}`. Reply `accessToken`, `refreshToken`.
- Refresh: `POST {base}/login` with `{"app": "OCM", "emailAddress", "refreshToken"}`. So the email must be stored with the token.
- Authenticated calls send `Authorization: <accessToken>` (no "Bearer").
- If the base URL contains `/app`, paths get a `/v2` prefix.
- Readings: `GET {base}{v2}/sync/bp?nextpaginationKey=0&lastSyncedTime=<ms or empty>&phoneIdentifier=`. Reply `{"data": [...]}`. Items: `userNumberInDevice`, `measurementDate` (ms), `timeZone` (offset in seconds, as string or number), `systolic`, `diastolic`, `pulse`, `irregularHB`, `movementDetect`, `cuffWrapDetect`, `isManualEntry` (skip when non-zero), `notes`. Read omramin `omronconnect.py` `OmronConnect2` for any details (pagination key, device model field) — facts only.
- Mark v2 as untested in the README and in the code's module doc.

## `omron-connect` library API (shape, not exact names)

- `Region::for_country(code) -> Result<Region>`: v1 or v2; error for unknown or offline countries.
- `Session` (serializable): region or country, email, access token, its expiry, refresh token. `Debug` hides tokens.
- `Client` built from a `reqwest::Client` the caller provides (so the caller controls TLS and timeouts) and a base-URL override for tests.
  - `login(country, email, password) -> Session`
  - `refresh(&mut Session)`; call automatically when the access token is missing or within 60 s of expiry.
  - `has_new(&Session, cursor) -> bool` (v1 uses `countOnlyFlag`; v2 always true).
  - `fetch_since(&Session, cursor) -> Batch { readings, deleted, cursor }`.
- `Reading`: `measured_at_ms`, the timezone as given (IANA name, or a fixed offset for v2), `systolic`, `diastolic`, `pulse: Option`, `irregular_heartbeat`, `body_movement`, `cuff_wrap_ok: Option<bool>`, `device_model`, `user_number`, and a stable private key (serial + user + time) for dedupe and deletes. The serial is never shown in tool output.
- Typed errors: bad credentials, token expired (re-login needed), unsupported country, rate limited, upstream error with status, reply too large.

## Memory

- The sync reply grows by ~220 KB per reading on pulse-wave cuffs, so a first sync of a long history can be tens or hundreds of MB. Peak memory must not grow with reply size.
- Approach: stream the response body chunks through a small bounded channel into `serde_json::from_reader` running in `spawn_blocking`. Use typed structs, and deserialize `bodyIndexList` with a custom visitor that keeps codes 1, 2, 3, 6, 7, 8 and skips every other value with `serde::de::IgnoredAny`. Any simpler approach that meets the test below is fine.
- Cap the total download (for example 1 GiB) and fail with "reply too large".
- Test: a fake server returns 100 readings, each with a 300 KB `ppwData`, served in chunks (about 30 MB total). A counting global allocator in that test binary asserts that peak heap stays under 4 MB.

## `omron-mcp` server

- `ServiceInfo`: command `omron-mcp`, env prefix `OMRON_MCP_`, default listen `127.0.0.1:8082` (garmin-mcp uses 8081 and mfp-mcp 8080 on the same LXC), data dir `/var/lib/omron-mcp`, scope `omron`, home page rendered from the README (copy mfp's `build.rs` approach).
- State section `omron` in `state.json`: the `Session`, the sync cursor, `login_rev`, and `login_invalid`. Fields use `skip_serializing_if`, add a flattened `extra`, and `Debug` hides secrets. The password is never stored.
- Reading cache: a separate file `<data_dir>/readings.json`, written atomically (temp file then rename) with mode 0600. Store compact readings only, sorted by time. Load it per tool call and drop it after, so idle RSS does not grow with history.
- CLI: `serve` (default), `login` (asks for country, email, and a hidden password via mcp-starter's `read_secret`, then saves the session and runs a first sync), `status` (no secrets: login state, country, cursor time, cached count, newest reading time), `set-password`, `revoke-tokens`, `help`. Errors and exit codes as in mfp-mcp-rs.
- Background: a keep-alive task on the work tracker refreshes the Omron token once every 24 h, cancelled by shutdown (garmin-mcp `run_keep_alive` pattern). Nothing else runs while idle.
- Tool `omron_get_blood_pressure` (read-only, idempotent):
  - Inputs: `start_date` and `end_date` (YYYY-MM-DD, default the last 30 days in `OMRON_MCP_TZ`, at most 366 days), and an optional `user` (cuff user number).
  - It first syncs: a count check, then a fetch if anything is new, then merge into the cache (deletes applied, duplicates by key ignored). If the sync fails, it returns cached readings plus a `note` naming the problem. If the login is invalid, the note says to run `omron-mcp login`.
  - Output: readings newest first, each with `date`, `time` (local time in the reading's own timezone, with seconds), `utc_offset`, `systolic`, `diastolic`, `pulse`, `irregular_heartbeat`, `body_movement`, `cuff_wrap_ok`, `device`, `user`. Plus a summary: count, average, min and max for systolic, diastolic and pulse.
  - One tool is enough. Keep the description short and factual (units, defaults, limits).
- Tests (mfp-mcp-rs style): an in-process axum fake of v1 and v2 using invented values; a tools/list snapshot; CLI tests driven in-process; a state round-trip; a loadcheck under the `testhooks` feature with the same gates as mfp-mcp-rs (binary ≤ 8 MB, idle RSS ≤ 8 MB, one idle thread, flat idle CPU); and a `live` feature test that reads `OMRON_EMAIL`, `OMRON_PASSWORD` and `OMRON_COUNTRY` and is never run by default.
- README (public, also the home page): what it is, the tool, commands, settings, a resource-use table with measured numbers, development commands, and credits. Credits say that omramin and aircon-chen/omron-connect-mcp documented the protocol (v1 sync endpoint), that this is an independent MIT implementation, and "Not affiliated with OMRON Healthcare".
- `LICENSE` (MIT, owner "Risan Bagja"), with `license = "MIT"` in both crates.
- Deploy kit `deploy/alpine/omron-mcp.initd` and `.confd`, as mfp-mcp-rs: supervise-daemon, user `omron`, `/var/lib/omron-mcp`, `OMRON_MCP_LISTEN=127.0.0.1:8082`, `OMRON_MCP_TZ=Asia/Jakarta`, `OMRON_MCP_PUBLIC_URL` (placeholder `https://omron.example.com`), `memory.high 24M` and `memory.max 32M`. Add a short `docs/setup.md` for adding it to an existing Alpine LXC with a Cloudflare tunnel.

## Done when

- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test` pass.
- The release musl binary builds, and its size and the loadcheck numbers are in the README.
- No fixture contains a real value (grep check above).
- Work is committed on `main` in small, logical commits. There is no remote, and nothing is pushed.
