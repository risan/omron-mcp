//! An in-process stand-in for the v1 (Kii) cloud.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

use super::{Call, Calls, EMAIL, PASSWORD, json_response, record, serve};

pub const SYNC_PATH_SUFFIX: &str = "/server-code/versions/current/synchronizeMeasureData";

struct State1 {
    next_token: u32,
    valid_access: String,
    sync_reply: Value,
    count_reply: Value,
    forced_sync_status: Option<u16>,
    reject_refresh: bool,
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State1>>,
    calls: Calls,
}

pub struct FakeV1 {
    pub base_url: String,
    shared: Shared,
}

impl FakeV1 {
    pub async fn start() -> FakeV1 {
        let shared = Shared {
            state: Arc::new(Mutex::new(State1 {
                next_token: 0,
                valid_access: String::new(),
                sync_reply: json!({ "returnedValue": {} }),
                count_reply: json!({ "returnedValue": { "countSyncData": 0 } }),
                forced_sync_status: None,
                reject_refresh: false,
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

        FakeV1 { base_url, shared }
    }

    pub fn set_sync_reply(&self, reply: Value) {
        self.shared.state.lock().unwrap().sync_reply = reply;
    }

    pub fn set_count_reply(&self, reply: Value) {
        self.shared.state.lock().unwrap().count_reply = reply;
    }

    pub fn force_sync_status(&self, status: u16) {
        self.shared.state.lock().unwrap().forced_sync_status = Some(status);
    }

    pub fn reject_refresh(&self) {
        self.shared.state.lock().unwrap().reject_refresh = true;
    }

    /// Makes every access token handed out so far stop working.
    pub fn revoke_access_tokens(&self) {
        self.shared.state.lock().unwrap().valid_access = "revoked".into();
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

async fn token(
    State(shared): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let mut state = shared.state.lock().unwrap();

    let allowed = if request["grant_type"] == "refresh_token" {
        !state.reject_refresh
            && request["refresh_token"]
                .as_str()
                .is_some_and(|token| token.starts_with("refresh-"))
    } else {
        request["username"] == EMAIL && request["password"] == PASSWORD
    };

    if !allowed {
        return json_response(400, &json!({ "error": "invalid_grant" }));
    }

    state.next_token += 1;
    state.valid_access = format!("access-{}", state.next_token);

    json_response(
        200,
        &json!({
            "access_token": state.valid_access,
            "refresh_token": format!("refresh-{}", state.next_token),
            "expires_in": 3600,
            "token_type": "bearer"
        }),
    )
}

async fn sync(State(shared): State<Shared>, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    record(&shared.calls, "POST", &uri, &headers, &body);

    let state = shared.state.lock().unwrap();
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    if bearer != Some(format!("Bearer {}", state.valid_access).as_str()) {
        return json_response(401, &json!({ "error": "unauthorized" }));
    }

    if let Some(status) = state.forced_sync_status {
        return json_response(status, &json!({ "error": "forced" }));
    }

    let request: Value = serde_json::from_slice(&body).unwrap_or_default();

    if request["countOnlyFlag"] == 1 {
        return json_response(200, &state.count_reply);
    }

    json_response(200, &state.sync_reply)
}
