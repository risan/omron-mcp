//! Shared helpers for the integration tests. Every value here is invented.

#![allow(dead_code)]

pub mod fake_v1;
pub mod fake_v2;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use omron_connect::Client;
use serde_json::{Value, json};

pub const EMAIL: &str = "reader@example.test";
pub const PASSWORD: &str = "fixture-password";
pub const SERIAL: &str = "TEST-SERIAL-0001";
pub const SECOND_SERIAL: &str = "TEST-SERIAL-0002";

/// 2023-11-14T22:13:20Z.
pub const T0: i64 = 1_700_000_000_000;
pub const MINUTE: i64 = 60_000;

pub fn http_client() -> reqwest::Client {
    static PROVIDER: Once = Once::new();

    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    reqwest::Client::new()
}

pub fn client_for(base_url: &str) -> Client {
    Client::new(http_client()).with_base_urls([base_url])
}

#[derive(Debug, Clone)]
pub struct Call {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Call {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("the request body is JSON")
    }
}

pub type Calls = Arc<Mutex<Vec<Call>>>;

pub fn record(
    calls: &Calls,
    method: &str,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
    body: &[u8],
) {
    let headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();

    calls.lock().unwrap().push(Call {
        method: method.to_owned(),
        path: uri.path().to_owned(),
        query: uri.query().unwrap_or_default().to_owned(),
        headers,
        body: String::from_utf8_lossy(body).into_owned(),
    });
}

pub fn json_response(status: u16, value: &Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

pub async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{address}")
}

/// A v1 blood pressure measure with invented numbers. The body index list
/// also carries codes the library must ignore.
pub fn bp_measure(measured_at_ms: i64, systolic: i64, diastolic: i64, pulse: i64) -> Value {
    json!({
        "timeZone": "Asia/Jakarta",
        "deleteFlag": 0,
        "transferDate": measured_at_ms + 5_000,
        "measureDateFrom": measured_at_ms,
        "measureDateTo": measured_at_ms,
        "measureDeviceDateFrom": "2023-11-15 05:13:20",
        "measureDeviceDateTo": "2023-11-15 05:13:20",
        "measurementMode": 0,
        "userUpdateDate": measured_at_ms + 5_000,
        "bodyIndexList": {
            "1": [systolic.to_string(), "mmHg", "0", "2001"],
            "2": [diastolic.to_string(), "mmHg", "0", "2002"],
            "3": [pulse.to_string(), "bpm", "0", "2003"],
            "6": ["0", "", "0", "2006"],
            "7": ["0", "", "0", "2007"],
            "8": ["1", "", "0", "2008"],
            "10": ["1", "", "0", "2010"],
            "35": ["0", "", "0", "2035"]
        }
    })
}

/// The pulse-wave twin of a reading: no codes 1 and 2, just `ppwData`.
pub fn pulse_wave_measure(measured_at_ms: i64, samples: usize) -> Value {
    let ppw: Vec<String> = (0..samples)
        .map(|sample| (sample % 997).to_string())
        .collect();

    json!({
        "timeZone": "Asia/Jakarta",
        "deleteFlag": 0,
        "measureDateFrom": measured_at_ms,
        "measureDateTo": measured_at_ms,
        "measurementMode": 0,
        "bodyIndexList": { "ppwData": ppw }
    })
}

/// One `syncList` entry holding the measures of a single device and user.
pub fn sync_item(
    category: &str,
    model: &str,
    serial: &str,
    user: i64,
    measures: Vec<Value>,
) -> Value {
    json!({
        "objectId": "fixture-object",
        "transferStartDate": T0,
        "deviceCategoryList": [{
            "deviceCategory": category,
            "deviceModelList": [{
                "deviceModel": model,
                "deviceSerialIDList": [{
                    "deviceSerialID": serial,
                    "userNumberInDevice": user,
                    "measureList": measures
                }]
            }]
        }]
    })
}

pub fn sync_reply(items: Vec<Value>, last_sync_date: Option<i64>) -> Value {
    let mut returned = json!({ "syncList": items });

    if let Some(last) = last_sync_date {
        returned["lastSyncDate"] = json!(last);
    }

    json!({ "returnedValue": returned })
}
