//! The v2 cloud is untested against a live account. This pins that the server works over the
//! library's v2 client against an invented fake: sign in, sync, and read through the tool.

mod support;

use std::sync::Arc;

use mcp_starter::config::Config;
use mcp_starter::test_support::serve_mcp;
use mcp_starter::work::Work;
use omron_mcp::SERVICE;
use omron_mcp::app::App;
use omron_mcp::sync::{KEEP_ALIVE_EVERY, Omron};
use omron_mcp::tools::{OmronTools, ToolOptions};
use serde_json::json;
use support::fake_v2::FakeV2;
use support::{EMAIL, MINUTE, NOW, PASSWORD, T0, TestClock, omron_client, open_store};
use tokio_util::task::TaskTracker;

#[tokio::test]
async fn a_v2_account_syncs_and_reads_with_its_fixed_offset() {
    let fake = FakeV2::start().await;
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let omron = Omron::new(
        omron_client(&fake.base_url),
        store.clone(),
        KEEP_ALIVE_EVERY,
    );

    fake.add(T0, 128, 84, 68);
    fake.add(T0 + MINUTE, 122, 79, 65);
    omron.sign_in("NL", EMAIL, PASSWORD).await.unwrap();

    let clock = TestClock::new(NOW);
    let tools = ToolOptions {
        now: clock.as_clock(),
        ..ToolOptions::default()
    };
    let config = Config::load(&SERVICE, &|_| String::new(), false).unwrap();
    let app = Arc::new(App::new(
        omron,
        config,
        tools,
        Work::new(TaskTracker::new()),
    ));
    let mcp = serve_mcp(OmronTools::factory(app)).await;
    let result = mcp.call("omron_get_blood_pressure", json!({})).await;

    assert!(!result.is_error, "{}", result.text);

    let rows = result.structured["readings"].as_array().unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["systolic"], 122);
    assert_eq!(rows[0]["utc_offset"], "+01:00");
    assert_eq!(rows[0]["date"], "2023-11-14");
    assert_eq!(rows[0]["time"], "23:14:20");
    assert!(result.structured.get("note").is_none());

    let state = store.read(|state| state.section.clone()).await.unwrap();

    assert_eq!(state.session.unwrap().country, "NL");
    assert!(state.cursor_ms >= T0);
    assert!(fake.calls().iter().any(|call| call.path == "/sync/bp"));
}
