//! The release resource check, including the peak-live-heap gate. Not part of `cargo test`:
//!
//!     scripts/build-release.sh --bench
//!     OMRON_MCP_BENCH_BIN=$PWD/target/bench/x86_64-unknown-linux-musl/release/omron-mcp \
//!         cargo test --release --test loadcheck -- --ignored --nocapture
//!
//! It starts the benchmark binary (built with `testhooks`) against the fake OMRON, with a
//! state file holding 50 clients and a year of rotated refresh tokens and a cache of 2,000
//! readings, and reads `/proc` at three points: idle, after a burst of 90 tool calls (each
//! round also has one reading to sync and queries a whole year of readings) and 10 refresh
//! rotations, and across 60 more idle seconds.

mod support;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{Timestamp, ToSpan};
use mcp_starter::store::{Change, Client, Token, TokenKind};
use mcp_starter::test_support::jsonrpc::McpClient;
use mcp_starter::test_support::oauth_flow::OAuthClient;
use omron_connect::{Reading, ReadingZone};
use omron_mcp::SERVICE;
use omron_mcp::cache::ReadingCache;
use omron_mcp::store::OmronState;
use omron_mcp::sync::{KEEP_ALIVE_EVERY, Omron};
use serde_json::json;
use sha2::{Digest, Sha256};
use support::fake_omron::{FakeOmron, FakeReading};
use support::{COUNTRY, EMAIL, PASSWORD, omron_client, open_store, wait_until};
use tokio::time::sleep;

const IDLE_WAIT: Duration = Duration::from_secs(70);
// The connection pool closes connections idle for 30 s, so the last one is gone within 60 s
// of the last OMRON call.
const DRAIN_WAIT: Duration = Duration::from_secs(70);
const CPU_WINDOW: Duration = Duration::from_secs(60);
// Measured peaks: 2,080,423 bytes in every run. The peak is the biggest reply, a year of
// about 920 readings built as JSON values; a typical year is smaller.
const MAX_HEAP_PEAK_BYTES: u64 = 2_500_000;
// Measured peaks of the resident set: 8.4 to 8.7 MB, set by the same reply.
const MAX_PEAK_RSS_KB: u64 = 10 * 1024;
const BEARER: &str = "bench-access-token";
const FIRST_REFRESH: &str = "bench-refresh-0";
const CACHED_READINGS: i64 = 2_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug)]
struct Sample {
    rss_kb: u64,
    peak_rss_kb: u64,
    threads: usize,
    cpu_ticks: u64,
}

fn hex_sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sample(pid: u32) -> Sample {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let kb = |name: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{name} missing in /proc/{pid}/status"))
    };
    let threads = std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .count();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // the fields after the command name, which may itself contain spaces and parentheses
    let after_command = &stat[stat.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = after_command.split_whitespace().collect();
    let cpu_ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();

    Sample {
        rss_kb: kb("VmRSS:"),
        peak_rss_kb: kb("VmHWM:"),
        threads,
        cpu_ticks,
    }
}

fn start_binary(
    binary: &Path,
    data_dir: &Path,
    fake: &FakeOmron,
) -> (Child, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
    let mut child = Command::new(binary)
        .arg("serve")
        .env_clear()
        .env("OMRON_MCP_DATA_DIR", data_dir)
        .env("OMRON_MCP_PUBLIC_URL", "https://omron.example.com")
        .env("OMRON_MCP_LISTEN", "127.0.0.1:0")
        .env("OMRON_MCP_TEST_BASE_URL", &fake.base_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the benchmark binary starts");
    let log = Arc::new(Mutex::new(String::new()));
    let sink = log.clone();
    let stderr = child.stderr.take().unwrap();

    let stderr_reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            sink.lock().unwrap().push_str(&format!("{line}\n"));
        }
    });

    (child, log, stderr_reader)
}

fn heap_counters(stderr: &str) -> (u64, u64) {
    let line = stderr
        .lines()
        .find(|line| line.starts_with("heap peak_bytes="))
        .expect("benchmark stderr includes heap counters");
    let mut fields = line.split_whitespace();

    assert_eq!(fields.next(), Some("heap"));

    let peak = fields
        .next()
        .and_then(|field| field.strip_prefix("peak_bytes="))
        .and_then(|value| value.parse().ok())
        .expect("heap line includes an integer peak_bytes value");
    let live = fields
        .next()
        .and_then(|field| field.strip_prefix("live_bytes="))
        .and_then(|value| value.parse().ok())
        .expect("heap line includes an integer live_bytes value");

    assert!(
        fields.next().is_none(),
        "unexpected fields in heap line: {line}"
    );

    (peak, live)
}

fn assert_heap_peak_within_limit(heap_peak_bytes: u64, limit_bytes: u64) {
    assert!(
        heap_peak_bytes <= limit_bytes,
        "heap peak {heap_peak_bytes} bytes is above {limit_bytes} bytes"
    );
}

#[test]
#[should_panic(expected = "heap peak 850000 bytes is above 800000 bytes")]
fn heap_gate_failure_reports_peak_and_limit() {
    assert_heap_peak_within_limit(850_000, 800_000);
}

fn cached_reading(index: i64, now_ms: i64) -> Reading {
    Reading {
        key: format!("seed-{index:05}"),
        measured_at_ms: now_ms - (index + 1) * (DAY_MS * 2 / 5),
        zone: ReadingZone::Named("Asia/Jakarta".to_string()),
        systolic: 110 + (index % 40) as u16,
        diastolic: 70 + (index % 25) as u16,
        pulse: Some(55 + (index % 30) as u16),
        irregular_heartbeat: index % 50 == 0,
        body_movement: false,
        cuff_wrap_ok: Some(true),
        device_model: "TEST-BP-1".to_string(),
        user_number: 1 + (index % 2) as u32,
    }
}

async fn seed(fake: &FakeOmron, data_dir: &Path, now_ms: i64) {
    let store = open_store(data_dir);
    let omron = Omron::new(
        omron_client(&fake.base_url),
        store.clone(),
        KEEP_ALIVE_EVERY,
    );

    omron.sign_in(COUNTRY, EMAIL, PASSWORD).await.unwrap();

    let readings = (0..CACHED_READINGS)
        .map(|index| cached_reading(index, now_ms))
        .collect();

    ReadingCache::new(&store).merge(readings, &[]).unwrap();
    seed_oauth(&store).await;
}

async fn seed_oauth(store: &mcp_starter::store::Store<OmronState>) {
    store
        .update(|state| {
            let far: Timestamp = "2099-01-01T00:00:00Z".parse().unwrap();
            let registered_at: Timestamp = "2026-10-01T00:00:00Z".parse().unwrap();

            state.owner.password_hash =
                "$2b$12$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ012345".to_string();

            for index in 0..50 {
                let id = format!("client-{index}");

                state.clients.insert(
                    id.clone(),
                    Client {
                        client_id: id,
                        client_name: "Claude".to_string(),
                        redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".to_string()],
                        registered_at,
                        last_used_at: Some(registered_at),
                        extra: Default::default(),
                    },
                );
            }

            let token = |kind, rotated| Token {
                kind,
                client_id: "client-0".to_string(),
                family: "family".to_string(),
                scope: "omron offline_access".to_string(),
                resource: "https://omron.example.com/mcp".to_string(),
                expires_at: far,
                rotated,
                extra: Default::default(),
            };

            for index in 0..365 {
                state.tokens.insert(
                    hex_sha256(&format!("rotated-{index}")),
                    token(TokenKind::Refresh, true),
                );
            }

            state
                .tokens
                .insert(hex_sha256(BEARER), token(TokenKind::Access, false));
            state
                .tokens
                .insert(hex_sha256(FIRST_REFRESH), token(TokenKind::Refresh, false));

            Change::Write(())
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "release resource check; see the module comment"]
async fn resource_use_of_the_benchmark_binary() {
    let binary = PathBuf::from(
        std::env::var("OMRON_MCP_BENCH_BIN")
            .expect("OMRON_MCP_BENCH_BIN must name the benchmark binary"),
    );

    assert!(binary.is_file(), "{} does not exist", binary.display());

    let fake = FakeOmron::start().await;
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let now = Timestamp::now();

    seed(&fake, &data_dir, now.as_millisecond()).await;

    let (mut child, log, stderr_reader) = start_binary(&binary, &data_dir, &fake);
    let pid = child.id();

    wait_until(Duration::from_secs(20), "the binary to listen", || {
        log.lock().unwrap().contains("listening on")
    })
    .await;

    let address = {
        let text = log.lock().unwrap();

        text.split("listening on ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap()
            .to_string()
    };

    wait_until(Duration::from_secs(20), "the start-up keep-alive", || {
        fake.token_calls("refresh") == 1
    })
    .await;

    // idle: after the start-up keep-alive
    sleep(Duration::from_secs(10)).await;
    let idle_start = sample(pid);
    sleep(IDLE_WAIT - Duration::from_secs(10)).await;
    let idle = sample(pid);

    // burst: 90 authenticated tool calls and 10 refresh rotations
    let mcp = McpClient::new(&format!("http://{address}/mcp")).with_bearer(BEARER);
    let today = now
        .to_zoned(jiff::tz::TimeZone::get("Asia/Jakarta").unwrap())
        .date();
    let year_ago = today.checked_sub(364.days()).unwrap().to_string();
    let month_ago = today.checked_sub(29.days()).unwrap().to_string();
    let mut returned = 0;

    for round in 0..10 {
        fake.add(FakeReading::new(
            now.as_millisecond() - 60_000 * (round + 1),
            128,
            84,
            66,
        ));

        let calls = [
            json!({"start_date": year_ago}),
            json!({}),
            json!({"start_date": year_ago, "user": 1}),
            json!({"start_date": year_ago, "user": 2}),
            json!({"start_date": month_ago}),
            json!({"start_date": year_ago, "end_date": today.to_string()}),
            json!({"start_date": month_ago, "user": 1}),
            json!({"start_date": year_ago}),
            json!({"start_date": year_ago}),
        ];

        for arguments in calls {
            let result = mcp.call("omron_get_blood_pressure", arguments).await;

            assert!(!result.is_error, "round {round}: {}", result.text);
            assert!(
                result.structured.get("note").is_none(),
                "round {round}: {}",
                result.text
            );

            returned = result.structured["summary"]["count"].as_u64().unwrap();
        }
    }

    let oauth = OAuthClient::new(&SERVICE, &format!("http://{address}"));
    let mut refresh_token = FIRST_REFRESH.to_string();

    for rotation in 0..10 {
        let (reply, tokens) = oauth.refresh("client-0", &refresh_token).await;

        assert_eq!(reply.status, 200, "rotation {rotation}: {}", reply.body);

        refresh_token = tokens.unwrap().refresh_token;
    }

    let after_burst = sample(pid);

    // the workload reached the fake: one fetch per new reading, a count check per call
    assert_eq!(fake.fetches().len(), 10);
    assert_eq!(fake.count_checks().len(), 90);
    assert!(returned > 500, "the last call returned {returned} readings");

    // the burst's connection drain, then 60 idle seconds of CPU
    sleep(DRAIN_WAIT).await;
    let drained = sample(pid);

    sleep(CPU_WINDOW).await;
    let end = sample(pid);
    let cpu_delta = end.cpu_ticks - drained.cpu_ticks;

    println!(
        "LOADCHECK binary_bytes={}",
        std::fs::metadata(&binary).unwrap().len()
    );
    println!(
        "LOADCHECK idle_rss_kb={} idle_peak_rss_kb={} idle_threads={}",
        idle.rss_kb, idle.peak_rss_kb, idle.threads
    );
    println!(
        "LOADCHECK idle_window_cpu_ticks={}",
        idle.cpu_ticks - idle_start.cpu_ticks
    );
    println!(
        "LOADCHECK burst_rss_kb={} burst_peak_rss_kb={} burst_threads={} last_call_readings={returned}",
        after_burst.rss_kb, after_burst.peak_rss_kb, after_burst.threads
    );
    println!(
        "LOADCHECK drained_rss_kb={} drained_threads={}",
        drained.rss_kb, drained.threads
    );
    println!("LOADCHECK idle_cpu_delta_ticks_over_60s={cpu_delta}");

    unsafe {
        assert_eq!(libc::kill(pid as i32, libc::SIGTERM), 0, "send SIGTERM");
    }
    let status = child.wait().expect("benchmark exits after SIGTERM");
    assert!(status.success(), "benchmark exited with {status}");
    stderr_reader
        .join()
        .expect("stderr reader exits with the benchmark");

    let stderr = log.lock().unwrap();
    let (heap_peak_bytes, heap_live_at_exit_bytes) = heap_counters(&stderr);

    println!(
        "LOADCHECK heap_peak_bytes={heap_peak_bytes} heap_live_at_exit_bytes={heap_live_at_exit_bytes}"
    );

    assert!(
        idle.rss_kb <= 8 * 1024,
        "idle RSS {} kB is above 8 MB",
        idle.rss_kb
    );
    assert!(
        after_burst.rss_kb <= 8 * 1024,
        "RSS after the burst {} kB is above 8 MB",
        after_burst.rss_kb
    );
    assert!(
        after_burst.peak_rss_kb <= MAX_PEAK_RSS_KB,
        "peak RSS {} kB is above {MAX_PEAK_RSS_KB} kB",
        after_burst.peak_rss_kb
    );
    assert_heap_peak_within_limit(heap_peak_bytes, MAX_HEAP_PEAK_BYTES);
    assert_eq!(
        cpu_delta, 0,
        "process CPU time changed over 60 idle seconds"
    );
    assert_eq!(idle.threads, 1, "threads when idle");
    assert_eq!(drained.threads, 1, "threads after the burst has drained");
}
