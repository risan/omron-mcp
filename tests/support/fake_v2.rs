//! An in-process stand-in for the untested v2 cloud: login, refresh and one readings call.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};

use super::{Call, Calls, EMAIL, PASSWORD, json_response, record, serve};

#[derive(Clone)]
struct Shared {
    readings: Arc<Mutex<Value>>,
    valid_access: Arc<Mutex<String>>,
    calls: Calls,
}

pub struct FakeV2 {
    pub base_url: String,
    shared: Shared,
}

impl FakeV2 {
    pub async fn start() -> FakeV2 {
        let shared = Shared {
            readings: Arc::new(Mutex::new(json!({ "data": [] }))),
            valid_access: Arc::new(Mutex::new(String::new())),
            calls: Calls::default(),
        };
        let app = Router::new()
            .route("/login", post(login))
            .route("/sync/bp", get(readings))
            .with_state(shared.clone());

        FakeV2 {
            base_url: serve(app).await,
            shared,
        }
    }

    /// One reading in the shape the notes describe; the zone is an offset in seconds.
    pub fn add(&self, measured_at_ms: i64, systolic: i64, diastolic: i64, pulse: i64) {
        let item = json!({
            "userNumberInDevice": 1,
            "measurementDate": measured_at_ms,
            "timeZone": "3600",
            "systolic": systolic,
            "diastolic": diastolic,
            "pulse": pulse,
            "irregularHB": 0,
            "movementDetect": 0,
            "cuffWrapDetect": 1,
            "isManualEntry": 0,
            "notes": ""
        });

        self.shared.readings.lock().unwrap()["data"]
            .as_array_mut()
            .unwrap()
            .push(item);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.shared.calls.lock().unwrap().clone()
    }
}

async fn login(
    State(shared): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let allowed = request.get("refreshToken").is_some()
        || (request["emailAddress"] == EMAIL && request["password"] == PASSWORD);

    if !allowed {
        return json_response(401, &json!({ "message": "denied" }));
    }

    *shared.valid_access.lock().unwrap() = "v2-access".to_string();

    json_response(
        200,
        &json!({ "accessToken": "v2-access", "refreshToken": "v2-refresh-1" }),
    )
}

async fn readings(State(shared): State<Shared>, uri: Uri, headers: HeaderMap) -> Response {
    record(&shared.calls, "GET", &uri, &headers, b"");

    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    if authorization != Some(shared.valid_access.lock().unwrap().as_str()) {
        return json_response(401, &json!({ "message": "unauthorized" }));
    }

    json_response(200, &shared.readings.lock().unwrap())
}
