//! The v2 cloud is untested against a live account: these tests pin the wire
//! details taken from the omramin notes, against an invented fake.

mod support;

use omron_connect::{Client, Error, ReadingZone, Session};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::fake_v2::FakeV2;
use support::{EMAIL, MINUTE, PASSWORD, T0, client_for, http_client};

async fn signed_in(fake: &FakeV2) -> (Client, Session) {
    let client = client_for(&fake.base_url);
    let session = client.login("DE", EMAIL, PASSWORD).await.unwrap();

    (client, session)
}

fn item(measured_at_ms: i64, systolic: i64, diastolic: i64, pulse: i64) -> Value {
    json!({
        "userNumberInDevice": 1,
        "measurementDate": measured_at_ms,
        "timeZone": "25200",
        "systolic": systolic,
        "diastolic": diastolic,
        "pulse": pulse,
        "irregularHB": 0,
        "movementDetect": 0,
        "cuffWrapDetect": 1,
        "isManualEntry": 0,
        "notes": ""
    })
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
async fn login_sends_a_compact_body_with_its_checksum() {
    let fake = FakeV2::start("").await;
    let (_, session) = signed_in(&fake).await;

    assert_eq!(session.country, "DE");
    assert_eq!(session.access_token, "v2-access-1");
    assert_eq!(session.refresh_token, "v2-refresh-1");
    assert_eq!(session.base_url, fake.base_url);

    let call = &fake.calls()[0];
    assert_eq!(call.path, "/login");
    assert_eq!(
        call.json(),
        json!({ "emailAddress": EMAIL, "password": PASSWORD, "country": "DE", "app": "OCM" })
    );
    assert!(
        !call.body.contains(": ") && !call.body.contains(", "),
        "the body must be compact: {}",
        call.body
    );
    assert_eq!(
        call.header("checksum"),
        Some(sha256_hex(&call.body).as_str())
    );
    assert!(
        call.header("user-agent")
            .unwrap()
            .starts_with("OMRON connect/8.2.1")
    );
    assert_eq!(call.header("authorization"), None);
}

#[tokio::test]
async fn login_with_bad_credentials_is_reported() {
    let fake = FakeV2::start("").await;
    let client = client_for(&fake.base_url);

    let error = client.login("DE", EMAIL, "wrong").await.unwrap_err();

    assert!(matches!(error, Error::BadCredentials), "{error:?}");
}

#[tokio::test]
async fn refresh_sends_the_email_and_refresh_token_with_a_checksum() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;

    client.refresh(&mut session).await.unwrap();

    assert_eq!(session.access_token, "v2-access-2");
    assert_eq!(session.refresh_token, "v2-refresh-2");

    let call = fake.calls().pop().unwrap();
    assert_eq!(call.path, "/login");
    assert_eq!(
        call.json(),
        json!({ "app": "OCM", "emailAddress": EMAIL, "refreshToken": "v2-refresh-1" })
    );
    assert_eq!(
        call.header("checksum"),
        Some(sha256_hex(&call.body).as_str())
    );
}

#[tokio::test]
async fn a_rejected_refresh_means_sign_in_again() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;
    fake.reject_refresh();

    let error = client.refresh(&mut session).await.unwrap_err();

    assert!(matches!(error, Error::TokenExpired), "{error:?}");
}

#[tokio::test]
async fn readings_are_fetched_with_the_token_alone_in_the_authorization_header() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_readings(json!({ "data": [item(T0, 125, 78, 66)] }));

    client.fetch_since(&mut session, 0).await.unwrap();

    let call = fake.calls_ending_with("/sync/bp").pop().unwrap();
    assert_eq!(call.method, "GET");
    assert_eq!(call.header("authorization"), Some("v2-access-1"));
    assert_eq!(call.header("checksum"), None);
    assert_eq!(
        call.query,
        "nextpaginationKey=0&lastSyncedTime=&phoneIdentifier="
    );
}

#[tokio::test]
async fn the_cursor_is_sent_as_last_synced_time() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;

    client.fetch_since(&mut session, T0).await.unwrap();

    let call = fake.calls_ending_with("/sync/bp").pop().unwrap();
    assert_eq!(
        call.query,
        format!("nextpaginationKey=0&lastSyncedTime={T0}&phoneIdentifier=")
    );
}

#[tokio::test]
async fn items_become_readings_and_the_cursor_follows_the_newest() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;
    let mut manual = item(T0 + MINUTE, 150, 95, 80);
    manual["isManualEntry"] = json!(1);
    let mut flagged = item(T0 + 2 * MINUTE, 118, 74, 59);
    flagged["irregularHB"] = json!("1");
    flagged["movementDetect"] = json!(1);
    flagged["cuffWrapDetect"] = json!(0);
    flagged["timeZone"] = json!(-18000);
    flagged["userNumberInDevice"] = json!("2");
    fake.set_readings(json!({ "data": [item(T0, 125, 78, 66), manual, flagged] }));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(batch.cursor, T0 + 2 * MINUTE);
    assert!(batch.deleted.is_empty());
    assert_eq!(batch.readings.len(), 2);

    let first = &batch.readings[0];
    assert_eq!(first.measured_at_ms, T0);
    assert_eq!(first.zone, ReadingZone::OffsetSeconds(25_200));
    assert_eq!(
        (first.systolic, first.diastolic, first.pulse),
        (125, 78, Some(66))
    );
    assert_eq!(first.cuff_wrap_ok, Some(true));
    assert_eq!(first.user_number, 1);

    let second = &batch.readings[1];
    assert_eq!(second.zone, ReadingZone::OffsetSeconds(-18_000));
    assert!(second.irregular_heartbeat);
    assert!(second.body_movement);
    assert_eq!(second.cuff_wrap_ok, Some(false));
    assert_eq!(second.user_number, 2);
    assert_ne!(first.key, second.key);
}

#[tokio::test]
async fn the_cursor_is_kept_when_nothing_is_new() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;

    let batch = client.fetch_since(&mut session, T0).await.unwrap();

    assert_eq!(batch.cursor, T0);
    assert!(batch.readings.is_empty());
}

#[tokio::test]
async fn an_app_root_puts_v2_in_front_of_the_sync_path() {
    let fake = FakeV2::start("/app").await;
    let (client, mut session) = signed_in(&fake).await;
    fake.set_readings(json!({ "data": [item(T0, 125, 78, 66)] }));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(batch.readings.len(), 1);
    assert_eq!(
        fake.calls_ending_with("/sync/bp")[0].path,
        "/app/v2/sync/bp"
    );
    assert_eq!(fake.calls_ending_with("/login")[0].path, "/app/login");
}

#[tokio::test]
async fn a_rejected_access_token_is_refreshed_and_the_call_retried() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;
    session.access_token = "stale".into();
    fake.set_readings(json!({ "data": [item(T0, 125, 78, 66)] }));

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    assert_eq!(batch.readings.len(), 1);
    assert_eq!(session.access_token, "v2-access-2");
    assert_eq!(fake.calls_ending_with("/sync/bp").len(), 2);
}

#[tokio::test]
async fn login_falls_back_to_the_secondary_server_when_the_primary_is_down() {
    let fake = FakeV2::start("").await;
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let down = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let client =
        omron_connect::Client::new(http_client()).with_base_urls([down, fake.base_url.clone()]);

    let session = client.login("DE", EMAIL, PASSWORD).await.unwrap();

    assert_eq!(session.base_url, fake.base_url);
}

#[tokio::test]
async fn has_new_is_always_yes_and_asks_nothing() {
    let fake = FakeV2::start("").await;
    let (client, mut session) = signed_in(&fake).await;
    let calls_before = fake.calls().len();

    assert!(client.has_new(&mut session, T0).await.unwrap());
    assert_eq!(fake.calls().len(), calls_before);
}

#[tokio::test]
async fn the_united_states_uses_the_v2_cloud_too() {
    let fake = FakeV2::start("").await;
    let client = client_for(&fake.base_url);

    let session = client.login("us", EMAIL, PASSWORD).await.unwrap();

    assert_eq!(session.country, "US");
    assert_eq!(fake.calls()[0].json()["country"], "US");
}
