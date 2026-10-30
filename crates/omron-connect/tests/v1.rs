mod support;

use omron_connect::{Client, Error, ReadingZone, Session};
use serde_json::json;
use support::fake_v1::{FakeV1, SYNC_PATH_SUFFIX};
use support::{
    EMAIL, MINUTE, PASSWORD, SECOND_SERIAL, SERIAL, T0, bp_measure, client_for, pulse_wave_measure,
    sync_item, sync_reply,
};

async fn signed_in(fake: &FakeV1) -> (Client, Session) {
    let client = client_for(&fake.base_url);
    let session = client.login("ID", EMAIL, PASSWORD).await.unwrap();

    (client, session)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[tokio::test]
async fn login_returns_a_session_and_sends_the_app_headers() {
    let fake = FakeV1::start().await;
    let (_, session) = signed_in(&fake).await;

    assert_eq!(session.country, "ID");
    assert_eq!(session.access_token, "access-1");
    assert_eq!(session.refresh_token, "refresh-1");
    assert!(session.access_expires_at_ms > now_ms() + 3_500_000);

    let call = &fake.calls()[0];
    assert_eq!(call.path, "/api/oauth2/token");
    assert_eq!(
        call.json(),
        json!({ "username": EMAIL, "password": PASSWORD })
    );
    assert_eq!(call.header("x-kii-appid"), Some("lou30y2xfa9f"));
    assert_eq!(
        call.header("x-kii-appkey"),
        Some("392a4bdff8af4141944d30ca8e3cc860")
    );
    assert_eq!(call.header("x-ogsc-sdk-version"), Some("000.005"));
    assert_eq!(call.header("x-ogsc-app-version"), Some("011.004.00000"));
    assert!(
        call.header("user-agent")
            .unwrap()
            .starts_with("OmronConnect/011.004.00000.001")
    );
    assert_eq!(call.header("content-type"), Some("application/json"));
    assert_eq!(call.header("accept-encoding"), None);
}

#[tokio::test]
async fn login_with_bad_credentials_is_reported() {
    let fake = FakeV1::start().await;
    let client = client_for(&fake.base_url);

    let error = client.login("ID", EMAIL, "wrong").await.unwrap_err();

    assert!(matches!(error, Error::BadCredentials), "{error:?}");
}

#[tokio::test]
async fn the_session_debug_output_hides_secrets() {
    let fake = FakeV1::start().await;
    let (_, session) = signed_in(&fake).await;

    let debug = format!("{session:?}");

    assert!(!debug.contains("access-1"));
    assert!(!debug.contains("refresh-1"));
    assert!(!debug.contains(EMAIL));
}

#[tokio::test]
async fn refresh_swaps_in_new_tokens() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;

    client.refresh(&mut session).await.unwrap();

    assert_eq!(session.access_token, "access-2");
    assert_eq!(session.refresh_token, "refresh-2");

    let call = fake.calls().pop().unwrap();
    assert_eq!(
        call.json(),
        json!({ "grant_type": "refresh_token", "refresh_token": "refresh-1" })
    );
}

#[tokio::test]
async fn a_rejected_refresh_means_sign_in_again() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.reject_refresh();

    let error = client.refresh(&mut session).await.unwrap_err();

    assert!(matches!(error, Error::TokenExpired), "{error:?}");
}

#[tokio::test]
async fn a_token_close_to_expiry_is_refreshed_before_the_call() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    session.access_expires_at_ms = now_ms() + 30_000;

    client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(session.access_token, "access-2");

    let sync = &fake.calls_ending_with(SYNC_PATH_SUFFIX)[0];
    assert_eq!(sync.header("authorization"), Some("Bearer access-2"));
}

#[tokio::test]
async fn a_fresh_token_is_not_refreshed() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;

    client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(fake.calls().len(), 2);
    assert_eq!(session.access_token, "access-1");
}

#[tokio::test]
async fn a_rejected_access_token_is_refreshed_and_the_call_retried_once() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.revoke_access_tokens();

    client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(session.access_token, "access-2");
    assert_eq!(fake.calls_ending_with(SYNC_PATH_SUFFIX).len(), 2);
}

#[tokio::test]
async fn an_access_token_that_keeps_failing_means_sign_in_again() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.revoke_access_tokens();
    fake.reject_refresh();

    let error = client.fetch_since(&mut session, 0).await.unwrap_err();

    assert!(matches!(error, Error::TokenExpired), "{error:?}");
}

#[tokio::test]
async fn has_new_asks_for_a_count_only() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;

    fake.set_count_reply(json!({ "returnedValue": { "lastSyncDate": T0, "countSyncData": 0 } }));
    assert!(!client.has_new(&mut session, T0).await.unwrap());

    fake.set_count_reply(json!({ "returnedValue": { "lastSyncDate": T0, "countSyncData": 3 } }));
    assert!(client.has_new(&mut session, T0).await.unwrap());

    let call = fake.calls_ending_with(SYNC_PATH_SUFFIX).pop().unwrap();
    assert_eq!(
        call.json(),
        json!({ "lastSyncDate": T0, "countOnlyFlag": 1 })
    );
    assert!(call.path.contains("/apps/lou30y2xfa9f/"));
}

#[tokio::test]
async fn has_new_says_yes_when_the_count_is_missing() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_count_reply(json!({ "returnedValue": {} }));

    assert!(client.has_new(&mut session, T0).await.unwrap());
}

#[tokio::test]
async fn fetch_returns_readings_and_the_new_cursor() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(sync_reply(
        vec![sync_item(
            "0",
            "TEST-MODEL-A",
            SERIAL,
            1,
            vec![bp_measure(T0, 123, 77, 64)],
        )],
        Some(T0 + 1),
    ));

    let batch = client.fetch_since(&mut session, 5).await.unwrap();

    assert_eq!(batch.cursor, T0 + 1);
    assert!(batch.deleted.is_empty());
    assert_eq!(batch.readings.len(), 1);

    let reading = &batch.readings[0];
    assert_eq!(reading.measured_at_ms, T0);
    assert_eq!(reading.zone, ReadingZone::Named("Asia/Jakarta".into()));
    assert_eq!(
        (reading.systolic, reading.diastolic, reading.pulse),
        (123, 77, Some(64))
    );
    assert!(!reading.irregular_heartbeat);
    assert!(!reading.body_movement);
    assert_eq!(reading.cuff_wrap_ok, Some(true));
    assert_eq!(reading.device_model, "TEST-MODEL-A");
    assert_eq!(reading.user_number, 1);
    assert!(!reading.key.contains(SERIAL));

    let call = fake.calls_ending_with(SYNC_PATH_SUFFIX).pop().unwrap();
    assert_eq!(
        call.json(),
        json!({ "lastSyncDate": 5, "countOnlyFlag": 0 })
    );
}

#[tokio::test]
async fn the_cursor_is_kept_when_the_reply_has_no_last_sync_date() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(json!({ "returnedValue": {} }));

    let batch = client.fetch_since(&mut session, T0).await.unwrap();

    assert_eq!(batch.cursor, T0);
    assert!(batch.readings.is_empty());
}

#[tokio::test]
async fn pulse_wave_records_are_skipped() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(sync_reply(
        vec![
            sync_item(
                "0",
                "TEST-MODEL-A",
                SERIAL,
                1,
                vec![bp_measure(T0, 123, 77, 64)],
            ),
            sync_item(
                "61440",
                "TEST-MODEL-A",
                SERIAL,
                1,
                vec![pulse_wave_measure(T0, 500)],
            ),
        ],
        Some(T0),
    ));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(batch.readings.len(), 1);
    assert!(batch.deleted.is_empty());
}

#[tokio::test]
async fn deleted_readings_are_reported_by_the_key_of_the_original() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(sync_reply(
        vec![sync_item(
            "0",
            "TEST-MODEL-A",
            SERIAL,
            1,
            vec![bp_measure(T0, 123, 77, 64)],
        )],
        Some(T0),
    ));
    let first = client.fetch_since(&mut session, 0).await.unwrap();

    let mut deleted = bp_measure(T0, 123, 77, 64);
    deleted["deleteFlag"] = json!(1);
    fake.set_sync_reply(sync_reply(
        vec![
            sync_item("0", "TEST-MODEL-A", SERIAL, 1, vec![deleted]),
            sync_item(
                "61440",
                "TEST-MODEL-A",
                SERIAL,
                1,
                vec![{
                    let mut wave = pulse_wave_measure(T0, 10);
                    wave["deleteFlag"] = json!(1);
                    wave
                }],
            ),
        ],
        Some(T0 + 1),
    ));
    let second = client.fetch_since(&mut session, T0).await.unwrap();

    assert!(second.readings.is_empty());
    assert_eq!(second.deleted, vec![first.readings[0].key.clone()]);
}

#[tokio::test]
async fn values_are_multiplied_by_ten_to_the_scale() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    let mut measure = bp_measure(T0, 0, 0, 0);
    measure["bodyIndexList"]["1"] = json!(["12", "mmHg", "1", "1"]);
    measure["bodyIndexList"]["2"] = json!(["800", "mmHg", "-1", "2"]);
    measure["bodyIndexList"]["3"] = json!(["66", "bpm", "0", "3"]);
    fake.set_sync_reply(sync_reply(
        vec![sync_item("0", "TEST-MODEL-A", SERIAL, 1, vec![measure])],
        Some(T0),
    ));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    let reading = &batch.readings[0];
    assert_eq!(
        (reading.systolic, reading.diastolic, reading.pulse),
        (120, 80, Some(66))
    );
}

#[tokio::test]
async fn flags_and_a_missing_pulse_are_read() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    let mut measure = bp_measure(T0, 133, 85, 70);
    measure["bodyIndexList"]["6"] = json!(["1", "", "0", "6"]);
    measure["bodyIndexList"]["7"] = json!(["1", "", "0", "7"]);
    measure["bodyIndexList"]["8"] = json!(["0", "", "0", "8"]);
    measure["bodyIndexList"]
        .as_object_mut()
        .unwrap()
        .remove("3");
    fake.set_sync_reply(sync_reply(
        vec![sync_item("0", "TEST-MODEL-A", SERIAL, 1, vec![measure])],
        Some(T0),
    ));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    let reading = &batch.readings[0];
    assert_eq!(reading.pulse, None);
    assert!(reading.irregular_heartbeat);
    assert!(reading.body_movement);
    assert_eq!(reading.cuff_wrap_ok, Some(false));
}

#[tokio::test]
async fn several_devices_and_users_each_keep_their_own_readings() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(sync_reply(
        vec![
            sync_item(
                "0",
                "TEST-MODEL-A",
                SERIAL,
                1,
                vec![
                    bp_measure(T0, 121, 76, 61),
                    bp_measure(T0 + MINUTE, 124, 79, 62),
                ],
            ),
            sync_item(
                "0",
                "TEST-MODEL-A",
                SERIAL,
                2,
                vec![bp_measure(T0, 131, 86, 71)],
            ),
            sync_item(
                "0",
                "TEST-MODEL-B",
                SECOND_SERIAL,
                1,
                vec![bp_measure(T0, 141, 91, 81)],
            ),
        ],
        Some(T0 + MINUTE),
    ));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(batch.readings.len(), 4);

    let users: Vec<u32> = batch
        .readings
        .iter()
        .map(|reading| reading.user_number)
        .collect();
    assert_eq!(users, vec![1, 1, 2, 1]);

    let mut keys: Vec<&str> = batch
        .readings
        .iter()
        .map(|reading| reading.key.as_str())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 4);
    assert_eq!(batch.readings[3].device_model, "TEST-MODEL-B");
}

#[tokio::test]
async fn unsupported_countries_fail_before_any_request() {
    let fake = FakeV1::start().await;
    let client = client_for(&fake.base_url);

    for country in ["KH", "SA", "ZZ"] {
        let error = client.login(country, EMAIL, PASSWORD).await.unwrap_err();

        assert!(
            matches!(error, Error::UnsupportedCountry(_)),
            "{country}: {error:?}"
        );
    }

    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn rate_limiting_and_server_errors_are_told_apart() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;

    fake.force_sync_status(429);
    let error = client.fetch_since(&mut session, 0).await.unwrap_err();
    assert!(matches!(error, Error::RateLimited), "{error:?}");

    fake.force_sync_status(503);
    let error = client.fetch_since(&mut session, 0).await.unwrap_err();
    assert!(
        matches!(error, Error::Upstream { status: 503 }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_reply_past_the_cap_is_refused() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    let client = client.with_max_reply_bytes(2_000);
    fake.set_sync_reply(sync_reply(
        vec![sync_item(
            "0",
            "TEST-MODEL-A",
            SERIAL,
            1,
            (0..50)
                .map(|n| bp_measure(T0 + n * MINUTE, 120, 80, 60))
                .collect(),
        )],
        Some(T0),
    ));

    let error = client.fetch_since(&mut session, 0).await.unwrap_err();

    assert!(matches!(error, Error::ReplyTooLarge), "{error:?}");
}

#[tokio::test]
async fn an_error_reply_from_the_sync_call_is_not_taken_for_no_news() {
    let fake = FakeV1::start().await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_sync_reply(json!({ "returnedValue": { "errorCode": "SERVER_ERROR" } }));

    let error = client.fetch_since(&mut session, 0).await.unwrap_err();

    assert!(matches!(error, Error::BadReply(_)), "{error:?}");
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = client_for(&format!("http://{}/api", closed.local_addr().unwrap()));
    drop(closed);

    let error = client.login("ID", EMAIL, PASSWORD).await.unwrap_err();

    assert!(matches!(error, Error::Network(_)), "{error:?}");
}
