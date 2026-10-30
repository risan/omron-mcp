//! An in-process stand-in for the v2 cloud.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};

use super::{Call, Calls, EMAIL, PASSWORD, json_response, record, serve};

struct State2 {
    next_token: u32,
    valid_access: String,
    readings: Value,
    reject_refresh: bool,
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State2>>,
    calls: Calls,
}

pub struct FakeV2 {
    pub base_url: String,
    shared: Shared,
}

impl FakeV2 {
    /// `prefix` is the path the API root lives under, such as `""` or `"/app"`.
    /// A root containing `/app` also puts `/v2` before the sync path.
    pub async fn start(prefix: &str) -> FakeV2 {
        let shared = Shared {
            state: Arc::new(Mutex::new(State2 {
                next_token: 0,
                valid_access: String::new(),
                readings: json!({ "data": [] }),
                reject_refresh: false,
            })),
            calls: Calls::default(),
        };
        let v2 = if prefix.contains("/app") { "/v2" } else { "" };
        let app = Router::new()
            .route(&format!("{prefix}/login"), post(login))
            .route(&format!("{prefix}{v2}/sync/bp"), get(readings))
            .with_state(shared.clone());
        let base_url = format!("{}{prefix}", serve(app).await);

        FakeV2 { base_url, shared }
    }

    pub fn set_readings(&self, reply: Value) {
        self.shared.state.lock().unwrap().readings = reply;
    }

    pub fn reject_refresh(&self) {
        self.shared.state.lock().unwrap().reject_refresh = true;
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
}

async fn login(
    State(shared): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let mut state = shared.state.lock().unwrap();

    let allowed = if request.get("refreshToken").is_some() {
        !state.reject_refresh
            && request["refreshToken"]
                .as_str()
                .is_some_and(|token| token.starts_with("v2-refresh-"))
    } else {
        request["emailAddress"] == EMAIL && request["password"] == PASSWORD
    };

    if !allowed {
        return json_response(401, &json!({ "message": "denied" }));
    }

    state.next_token += 1;
    state.valid_access = format!("v2-access-{}", state.next_token);

    json_response(
        200,
        &json!({
            "accessToken": state.valid_access,
            "refreshToken": format!("v2-refresh-{}", state.next_token)
        }),
    )
}

async fn readings(State(shared): State<Shared>, uri: Uri, headers: HeaderMap) -> Response {
    record(&shared.calls, "GET", &uri, &headers, b"");

    let state = shared.state.lock().unwrap();
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    if authorization != Some(state.valid_access.as_str()) {
        return json_response(401, &json!({ "message": "unauthorized" }));
    }

    json_response(200, &state.readings)
}
