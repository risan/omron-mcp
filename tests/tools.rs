//! `omron_get_blood_pressure` against the in-process fake OMRON.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use serde_json::json;
use support::fake_omron::FakeReading;
use support::{DAY, MINUTE, T0, ToolsEnv, ToolsSetup, rows, wait_until};

/// Three readings on Jakarta days 2023-11-15 and 2023-11-16.
fn three_readings(rig: &ToolsEnv) {
    let fake = rig.fake();

    fake.add(FakeReading::new(T0, 132, 86, 71));
    fake.add(FakeReading::new(T0 + 5 * MINUTE, 126, 82, 69));
    fake.add(FakeReading::new(T0 + DAY, 141, 91, 77).irregular());
}

#[tokio::test]
async fn syncs_first_then_reads_the_cache() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);

    let result = rig.blood_pressure(json!({})).await;

    assert!(!result.is_error, "{}", result.text);

    let rows = rows(&result);

    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json!({
            "date": "2023-11-16", "time": "05:13:20", "utc_offset": "+07:00",
            "systolic": 141, "diastolic": 91, "pulse": 77,
            "irregular_heartbeat": true, "body_movement": false, "cuff_wrap_ok": true,
            "device": "TEST-BP-1", "user": 1
        })
    );
    assert_eq!(rows[1]["time"], "05:18:20");
    assert_eq!(rows[2]["time"], "05:13:20");
    assert_eq!(rows[2]["date"], "2023-11-15");
    assert!(result.structured.get("note").is_none());
    assert!(
        !result.text.contains("TEST-SERIAL"),
        "the serial is never shown"
    );
    assert_eq!(rig.fake().count_checks().len(), 1);
    assert_eq!(rig.fake().fetches().len(), 1);
}

#[tokio::test]
async fn the_cache_file_is_private_and_holds_no_serial_number() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;

    let path = rig.env.cache_path();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let text = std::fs::read_to_string(&path).unwrap();

    assert_eq!(mode, 0o600);
    assert!(!text.contains("TEST-SERIAL"));
    assert_eq!(rig.env.cached().len(), 3);
}

#[tokio::test]
async fn the_cursor_is_saved_and_only_new_readings_are_fetched() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;

    let cursor = rig.env.section().await.cursor_ms;

    assert!(cursor > T0, "cursor {cursor}");

    rig.blood_pressure(json!({})).await;

    assert_eq!(rig.fake().count_checks().len(), 2);
    assert_eq!(
        rig.fake().fetches().len(),
        1,
        "nothing is new, so the second call only counts"
    );
    assert_eq!(rig.fake().count_checks()[1].json()["lastSyncDate"], cursor);

    rig.fake().add(FakeReading::new(T0 + 2 * DAY, 120, 78, 66));

    let result = rig.blood_pressure(json!({})).await;

    assert_eq!(rows(&result).len(), 4);
    assert_eq!(rig.fake().fetches().len(), 2);
    assert_eq!(rig.fake().fetches()[1].json()["lastSyncDate"], cursor);
    assert!(rig.env.section().await.cursor_ms > cursor);
}

#[tokio::test]
async fn a_reading_deleted_in_the_app_leaves_the_cache() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;
    rig.fake().delete(1);

    let result = rig.blood_pressure(json!({})).await;
    let systolic: Vec<i64> = rows(&result)
        .iter()
        .map(|row| row["systolic"].as_i64().unwrap())
        .collect();

    assert_eq!(systolic, [141, 132]);
    assert_eq!(rig.env.cached().len(), 2);
}

#[tokio::test]
async fn a_reading_delivered_twice_is_kept_once() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;
    rig.fake().resend(0);
    rig.fake().resend(2);

    let result = rig.blood_pressure(json!({})).await;

    assert_eq!(rows(&result).len(), 3);
    assert_eq!(rig.fake().fetches().len(), 2, "the resend was fetched");
    assert_eq!(rig.env.cached().len(), 3);
}

#[tokio::test]
async fn the_user_filter_keeps_one_cuff_slot() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.fake()
        .add(FakeReading::new(T0 + 10 * MINUTE, 118, 74, 64).user(2));

    let all = rig.blood_pressure(json!({})).await;
    let second = rig.blood_pressure(json!({"user": 2})).await;
    let first = rig.blood_pressure(json!({"user": 1})).await;
    let nobody = rig.blood_pressure(json!({"user": 3})).await;

    assert_eq!(rows(&all).len(), 4);
    assert_eq!(rows(&second).len(), 1);
    assert_eq!(rows(&second)[0]["user"], 2);
    assert_eq!(rows(&first).len(), 3);
    assert_eq!(rows(&nobody).len(), 0);
    assert_eq!(nobody.structured["summary"], json!({"count": 0}));
}

#[tokio::test]
async fn the_date_range_follows_each_readings_own_zone() {
    let rig = ToolsEnv::start().await;
    let fake = rig.fake();

    // 2023-11-14T22:13:20Z: the 14th in Amsterdam (+01:00), the 15th in Jakarta (+07:00).
    fake.add(FakeReading::new(T0, 130, 85, 70).zone("Europe/Amsterdam"));
    fake.add(FakeReading::new(T0 + MINUTE, 131, 86, 71));

    let fourteenth = rig
        .blood_pressure(json!({"start_date": "2023-11-14", "end_date": "2023-11-14"}))
        .await;
    let fifteenth = rig
        .blood_pressure(json!({"start_date": "2023-11-15", "end_date": "2023-11-15"}))
        .await;

    assert_eq!(rows(&fourteenth).len(), 1);
    assert_eq!(rows(&fourteenth)[0]["date"], "2023-11-14");
    assert_eq!(rows(&fourteenth)[0]["time"], "23:13:20");
    assert_eq!(rows(&fourteenth)[0]["utc_offset"], "+01:00");
    assert_eq!(rows(&fifteenth).len(), 1);
    assert_eq!(rows(&fifteenth)[0]["utc_offset"], "+07:00");
}

#[tokio::test]
async fn the_default_range_is_the_last_thirty_days() {
    let rig = ToolsEnv::start().await;
    let fake = rig.fake();

    // Today is 2023-11-20, so the range starts on 2023-10-22.
    fake.add(FakeReading::new(T0 - 20 * DAY, 120, 80, 60));
    fake.add(FakeReading::new(T0 - 40 * DAY, 121, 81, 61));
    fake.add(FakeReading::new(T0, 122, 82, 62));

    let result = rig.blood_pressure(json!({})).await;

    assert_eq!(result.structured["start_date"], "2023-10-22");
    assert_eq!(result.structured["end_date"], "2023-11-20");
    assert_eq!(rows(&result).len(), 2);
}

#[tokio::test]
async fn a_bad_range_is_refused_before_any_sync() {
    let rig = ToolsEnv::start().await;
    let cases = [
        (
            json!({"start_date": "2022-01-01", "end_date": "2023-12-31"}),
            "at most 366 days",
        ),
        (
            json!({"start_date": "2023-11-20", "end_date": "2023-11-01"}),
            "must not be after",
        ),
        (json!({"start_date": "yesterday"}), "use YYYY-MM-DD"),
        (json!({"end_date": "2023-02-30"}), "use YYYY-MM-DD"),
    ];

    for (arguments, expected) in cases {
        let result = rig.blood_pressure(arguments.clone()).await;

        assert!(result.is_error, "{arguments}");
        assert!(
            result.text.contains(expected),
            "{arguments}: {}",
            result.text
        );
    }

    assert!(
        rig.fake()
            .calls()
            .iter()
            .all(|call| !call.path.contains("synchronize"))
    );

    let limit = rig
        .blood_pressure(json!({"start_date": "2022-11-20", "end_date": "2023-11-20"}))
        .await;

    assert!(!limit.is_error, "366 days is allowed: {}", limit.text);
}

#[tokio::test]
async fn the_summary_has_the_count_average_min_and_max() {
    let rig = ToolsEnv::start().await;
    let fake = rig.fake();

    fake.add(FakeReading::new(T0, 120, 80, 60));
    fake.add(FakeReading::new(T0 + MINUTE, 131, 85, 71).irregular());
    fake.add(FakeReading::new(T0 + 2 * MINUTE, 140, 90, 70));
    fake.add(FakeReading::new(T0 + 3 * MINUTE, 125, 83, 0).without_pulse());

    let result = rig.blood_pressure(json!({})).await;

    assert_eq!(
        result.structured["summary"],
        json!({
            "count": 4,
            "systolic": {"average": 129.0, "min": 120, "max": 140},
            "diastolic": {"average": 84.5, "min": 80, "max": 90},
            "pulse": {"average": 67.0, "min": 60, "max": 71},
            "irregular_heartbeat_count": 1
        })
    );
    assert_eq!(rows(&result)[0]["pulse"], serde_json::Value::Null);
    assert!(
        result
            .text
            .starts_with("4 readings from 2023-10-22 to 2023-11-20: average 129/84.5 mmHg.")
    );
}

#[tokio::test]
async fn an_expired_login_answers_from_the_cache_and_names_the_fix() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;
    rig.fake().expire_access_tokens();
    rig.fake().refuse_refresh();
    rig.fake().add(FakeReading::new(T0 + 2 * DAY, 119, 77, 65));

    let result = rig.blood_pressure(json!({})).await;

    assert!(!result.is_error, "{}", result.text);
    assert_eq!(rows(&result).len(), 3);

    let note = result.structured["note"].as_str().unwrap();

    assert!(note.contains("omron-mcp login"), "{note}");
    assert!(rig.env.section().await.login_invalid);

    let calls = rig.fake().calls().len();
    let again = rig.blood_pressure(json!({})).await;

    assert!(
        again.structured["note"]
            .as_str()
            .unwrap()
            .contains("omron-mcp login")
    );
    assert_eq!(rig.fake().calls().len(), calls, "OMRON is not called again");
}

#[tokio::test]
async fn without_a_login_the_tool_says_to_sign_in() {
    let rig = ToolsEnv::start_with(ToolsSetup {
        signed_in: false,
        ..ToolsSetup::default()
    })
    .await;

    let result = rig.blood_pressure(json!({})).await;

    assert!(!result.is_error, "{}", result.text);
    assert!(rows(&result).is_empty());
    assert!(
        result.structured["note"]
            .as_str()
            .unwrap()
            .contains("omron-mcp login")
    );
    assert!(rig.fake().calls().is_empty());
}

#[tokio::test]
async fn an_upstream_failure_keeps_the_cached_readings_and_the_cursor() {
    let rig = ToolsEnv::start().await;

    three_readings(&rig);
    rig.blood_pressure(json!({})).await;

    let cursor = rig.env.section().await.cursor_ms;

    rig.fake().add(FakeReading::new(T0 + 2 * DAY, 119, 77, 65));
    rig.fake().force_status(500);

    let result = rig.blood_pressure(json!({})).await;
    let note = result.structured["note"].as_str().unwrap();

    assert!(note.contains("HTTP 500"), "{note}");
    assert_eq!(rows(&result).len(), 3);
    assert_eq!(rig.env.section().await.cursor_ms, cursor);
    assert!(!rig.env.section().await.login_invalid);
}

#[tokio::test]
async fn a_refreshed_token_is_saved_after_the_call() {
    let rig = ToolsEnv::start().await;
    let before = rig.env.section().await.session.unwrap();

    rig.fake().expire_access_tokens();
    three_readings(&rig);

    let result = rig.blood_pressure(json!({})).await;
    let after = rig.env.section().await.session.unwrap();

    assert!(result.structured.get("note").is_none(), "{}", result.text);
    assert_ne!(after.access_token, before.access_token);
    assert_ne!(after.refresh_token, before.refresh_token);
    assert_eq!(rig.fake().token_calls("refresh"), 1);
}

#[tokio::test]
async fn a_slow_sync_answers_from_the_cache_and_finishes_in_the_background() {
    let rig = ToolsEnv::start_with(ToolsSetup {
        tool_deadline: Duration::from_secs(5),
        sync_wait: Duration::from_millis(200),
        ..ToolsSetup::default()
    })
    .await;

    three_readings(&rig);
    rig.fake().delay_sync(Duration::from_millis(800));

    let started = Instant::now();
    let result = rig.blood_pressure(json!({})).await;

    assert!(
        started.elapsed() < Duration::from_millis(700),
        "{:?}",
        started.elapsed()
    );
    assert!(!result.is_error, "{}", result.text);
    assert!(rows(&result).is_empty());
    assert!(
        result.structured["note"]
            .as_str()
            .unwrap()
            .contains("still running")
    );

    wait_until(Duration::from_secs(5), "the background sync", || {
        rig.env.cached().len() == 3
    })
    .await;

    rig.fake().delay_sync(Duration::ZERO);

    let later = rig.blood_pressure(json!({})).await;

    assert_eq!(rows(&later).len(), 3);
    assert!(later.structured.get("note").is_none());
}
