//! An in-process stand-in for the OMRON v1 cloud that keeps readings and answers the sync
//! call by cursor. Every value is invented.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

use super::{Call, Calls, EMAIL, PASSWORD, T0, json_response, record, serve};

pub struct FakeReading {
    pub measured_at_ms: i64,
    pub systolic: i64,
    pub diastolic: i64,
    pub pulse: Option<i64>,
    pub irregular: bool,
    pub user: i64,
    pub zone: &'static str,
    pub serial: &'static str,
    pub transfer_date: i64,
    pub deleted: bool,
}

impl FakeReading {
    pub fn new(measured_at_ms: i64, systolic: i64, diastolic: i64, pulse: i64) -> FakeReading {
        FakeReading {
            measured_at_ms,
            systolic,
            diastolic,
            pulse: Some(pulse),
            irregular: false,
            user: 1,
            zone: "Asia/Jakarta",
            serial: "TEST-SERIAL-0001",
            transfer_date: 0,
            deleted: false,
        }
    }

    pub fn user(mut self, user: i64) -> FakeReading {
        self.user = user;
        self
    }

    pub fn zone(mut self, zone: &'static str) -> FakeReading {
        self.zone = zone;
        self
    }

    pub fn irregular(mut self) -> FakeReading {
        self.irregular = true;
        self
    }

    pub fn without_pulse(mut self) -> FakeReading {
        self.pulse = None;
        self
    }

    fn measure(&self) -> Value {
        let mut body = json!({
            "1": [self.systolic.to_string(), "mmHg", "0", "2001"],
            "2": [self.diastolic.to_string(), "mmHg", "0", "2002"],
            "6": [i64::from(self.irregular).to_string(), "", "0", "2006"],
            "7": ["0", "", "0", "2007"],
            "8": ["1", "", "0", "2008"],
            "10": ["1", "", "0", "2010"]
        });

        if let Some(pulse) = self.pulse {
            body["3"] = json!([pulse.to_string(), "bpm", "0", "2003"]);
        }

        json!({
            "timeZone": self.zone,
            "deleteFlag": i64::from(self.deleted),
            "transferDate": self.transfer_date,
            "measureDateFrom": self.measured_at_ms,
            "measureDateTo": self.measured_at_ms,
            "measurementMode": 0,
            "userUpdateDate": self.transfer_date,
            "bodyIndexList": body
        })
    }

    fn sync_item(&self) -> Value {
        json!({
            "objectId": "fixture-object",
            "transferStartDate": self.transfer_date,
            "deviceCategoryList": [{
                "deviceCategory": "0",
                "deviceModelList": [{
                    "deviceModel": "TEST-BP-1",
                    "deviceSerialIDList": [{
                        "deviceSerialID": self.serial,
                        "userNumberInDevice": self.user,
                        "measureList": [self.measure()]
                    }]
                }]
            }]
        })
    }
}

struct Data {
    readings: Vec<FakeReading>,
    transfer_clock: i64,
    next_token: u32,
    valid_access: String,
    refresh_works: bool,
    forced_status: Option<u16>,
    sync_delay: Duration,
}

#[derive(Clone)]
struct Shared {
    data: Arc<Mutex<Data>>,
    calls: Calls,
}

pub struct FakeOmron {
    pub base_url: String,
    shared: Shared,
}

impl FakeOmron {
    pub async fn start() -> FakeOmron {
        let shared = Shared {
            data: Arc::new(Mutex::new(Data {
                readings: Vec::new(),
                transfer_clock: T0,
                next_token: 0,
                valid_access: String::new(),
                refresh_works: true,
                forced_status: None,
                sync_delay: Duration::ZERO,
            })),
            calls: Calls::default(),
        };
        let app = Router::new()
            .route("/api/oauth2/token", post(token))
            .route(
                "/api/apps/{app_id}/server-code/versions/current/synchronizeMeasureData",
                post(sync),
            )
            .with_state(shared.clone());
        let base_url = format!("{}/api", serve(app).await);

        FakeOmron { base_url, shared }
    }

    /// Adds a reading as if the phone had just uploaded it; returns its index.
    pub fn add(&self, mut reading: FakeReading) -> usize {
        let mut data = self.shared.data.lock().unwrap();

        data.transfer_clock += 1_000;
        reading.transfer_date = data.transfer_clock;
        data.readings.push(reading);

        data.readings.len() - 1
    }

    /// The app deleted the reading: the next sync delivers it with a delete flag.
    pub fn delete(&self, index: usize) {
        let mut data = self.shared.data.lock().unwrap();

        data.transfer_clock += 1_000;

        let clock = data.transfer_clock;
        let reading = &mut data.readings[index];

        reading.deleted = true;
        reading.transfer_date = clock;
    }

    /// The same reading is delivered again by the next sync.
    pub fn resend(&self, index: usize) {
        let mut data = self.shared.data.lock().unwrap();

        data.transfer_clock += 1_000;

        let clock = data.transfer_clock;

        data.readings[index].transfer_date = clock;
    }

    pub fn expire_access_tokens(&self) {
        self.shared.data.lock().unwrap().valid_access = "expired".to_string();
    }

    pub fn refuse_refresh(&self) {
        self.shared.data.lock().unwrap().refresh_works = false;
    }

    pub fn accept_refresh(&self) {
        self.shared.data.lock().unwrap().refresh_works = true;
    }

    pub fn force_status(&self, status: u16) {
        self.shared.data.lock().unwrap().forced_status = Some(status);
    }

    pub fn delay_sync(&self, delay: Duration) {
        self.shared.data.lock().unwrap().sync_delay = delay;
    }

    pub fn calls(&self) -> Vec<Call> {
        self.shared.calls.lock().unwrap().clone()
    }

    pub fn calls_ending_with(&self, suffix: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.path.ends_with(suffix))
            .collect()
    }

    /// The sync calls that asked for readings, not just for a count.
    pub fn fetches(&self) -> Vec<Call> {
        self.calls_ending_with("synchronizeMeasureData")
            .into_iter()
            .filter(|call| call.json()["countOnlyFlag"] == 0)
            .collect()
    }

    pub fn count_checks(&self) -> Vec<Call> {
        self.calls_ending_with("synchronizeMeasureData")
            .into_iter()
            .filter(|call| call.json()["countOnlyFlag"] == 1)
            .collect()
    }

    pub fn token_calls(&self, grant: &str) -> usize {
        self.calls_ending_with("/oauth2/token")
            .iter()
            .filter(|call| {
                let is_refresh = call.json()["grant_type"] == "refresh_token";

                is_refresh == (grant == "refresh")
            })
            .count()
    }
}

async fn token(
    State(shared): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let mut data = shared.data.lock().unwrap();
    let allowed = if request["grant_type"] == "refresh_token" {
        data.refresh_works
            && request["refresh_token"]
                .as_str()
                .is_some_and(|token| token.starts_with("refresh-"))
    } else {
        request["username"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(EMAIL))
            && request["password"] == PASSWORD
    };

    if !allowed {
        return json_response(400, &json!({ "error": "invalid_grant" }));
    }

    data.next_token += 1;
    data.valid_access = format!("access-{}", data.next_token);

    json_response(
        200,
        &json!({
            "access_token": data.valid_access,
            "refresh_token": format!("refresh-{}", data.next_token),
            "expires_in": 3600,
            "token_type": "bearer"
        }),
    )
}

async fn sync(State(shared): State<Shared>, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let count_only = request["countOnlyFlag"] == 1;
    let delay = shared.data.lock().unwrap().sync_delay;

    if !count_only && !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    let data = shared.data.lock().unwrap();
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    if bearer != Some(format!("Bearer {}", data.valid_access).as_str()) {
        return json_response(401, &json!({ "error": "unauthorized" }));
    }

    if let Some(status) = data.forced_status {
        return json_response(status, &json!({ "error": "forced" }));
    }

    let cursor = request["lastSyncDate"].as_i64().unwrap_or(0);
    let newer: Vec<&FakeReading> = data
        .readings
        .iter()
        .filter(|reading| reading.transfer_date > cursor)
        .collect();
    let newest = newer.iter().map(|reading| reading.transfer_date).max();

    if count_only {
        return json_response(
            200,
            &json!({ "returnedValue": {
                "lastSyncDate": newest.unwrap_or(cursor),
                "countSyncData": newer.len()
            }}),
        );
    }

    let items: Vec<Value> = newer.iter().map(|reading| reading.sync_item()).collect();
    let mut returned = json!({ "syncList": items });

    if let Some(newest) = newest {
        returned["lastSyncDate"] = json!(newest);
    }

    json_response(200, &json!({ "returnedValue": returned }))
}
