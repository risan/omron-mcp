//! The v2 REST cloud, which serves Europe and North America.
//!
//! UNTESTED against a live account. The wire details come from the omramin
//! project's notes and have never been run against OMRON's servers by this
//! crate. Unverified points: whether the access token has a fixed lifetime
//! (an hour is assumed, and a 401 triggers a refresh anyway), whether the
//! secondary URL needs different handling, whether `lastSyncedTime` filters on
//! the measurement time or the upload time, and whether long histories are
//! paged (only the first page is read).

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{RequestBuilder, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::http::{Http, SMALL_REPLY_BYTES, check_status, read_json, read_json_streaming, send};
use crate::reading::{Batch, Reading, ReadingZone, reading_key};
use crate::session::{Session, Tokens};
use crate::wire::{Num, nonzero};

const APP_USER_AGENT: &str =
    "OMRON connect/8.2.1 (com.omronhealthcare.omronconnect; build:24; iOS 18.7.2) Alamofire/5.9.1";
const APP_NAME: &str = "OCM";
const DEFAULT_EXPIRES_IN_SECS: i64 = 3600;
const CREDENTIAL_STATUSES: &[u16] = &[400, 401, 403];
const MAX_READINGS_REPLY_BYTES: u64 = 64 * 1024 * 1024;

/// Some roots (the `/app` ones) put `/v2` in front of every path.
fn path_prefix(base_url: &str) -> &'static str {
    if base_url.contains("/app") { "/v2" } else { "" }
}

/// Posts a compact JSON body with the checksum header the API requires.
fn post(
    http: &Http,
    url: &str,
    body: &serde_json::Value,
    authorization: Option<&str>,
) -> RequestBuilder {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let checksum: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut request = http
        .client
        .post(url)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(CONTENT_TYPE, "application/json")
        .header("Checksum", checksum)
        .body(bytes);

    if let Some(token) = authorization {
        request = request.header(AUTHORIZATION, token);
    }

    request
}

/// Signs in on the first base URL that answers, trying the next one after a
/// connection failure or a server error. Returns the URL that worked.
pub(crate) async fn login(
    http: &Http,
    base_urls: &[String],
    country: &str,
    email: &str,
    password: &str,
) -> Result<(String, Tokens)> {
    let body = serde_json::json!({
        "emailAddress": email,
        "password": password,
        "country": country,
        "app": APP_NAME,
    });
    let mut last_error = Error::Network("no OMRON server is configured".into());

    for base_url in base_urls {
        match send_login(http, base_url, &body, Error::BadCredentials, None).await {
            Ok(tokens) => return Ok((base_url.clone(), tokens)),
            Err(error) if can_try_next_server(&error) => last_error = error,
            Err(error) => return Err(error),
        }
    }

    Err(last_error)
}

pub(crate) async fn refresh(http: &Http, session: &Session) -> Result<Tokens> {
    let body = serde_json::json!({
        "app": APP_NAME,
        "emailAddress": session.email,
        "refreshToken": session.refresh_token,
    });
    let authorization = Some(session.access_token.as_str()).filter(|token| !token.is_empty());

    send_login(
        http,
        &session.base_url,
        &body,
        Error::TokenExpired,
        authorization,
    )
    .await
}

fn can_try_next_server(error: &Error) -> bool {
    matches!(error, Error::Network(_))
        || matches!(error, Error::Upstream { status } if *status >= 500)
}

async fn send_login(
    http: &Http,
    base_url: &str,
    body: &serde_json::Value,
    credential_error: Error,
    authorization: Option<&str>,
) -> Result<Tokens> {
    let response = send(post(
        http,
        &format!("{base_url}/login"),
        body,
        authorization,
    ))
    .await?;
    check_status(&response, CREDENTIAL_STATUSES, credential_error)?;

    let reply: TokenReply = read_json(response, SMALL_REPLY_BYTES).await?;

    if reply.access_token.is_empty() || reply.refresh_token.is_empty() {
        return Err(Error::BadReply("the sign-in reply has no tokens".into()));
    }

    Ok(Tokens {
        access_token: reply.access_token,
        refresh_token: reply.refresh_token,
        expires_in_secs: DEFAULT_EXPIRES_IN_SECS,
    })
}

pub(crate) async fn fetch(http: &Http, session: &Session, cursor: i64) -> Result<Batch> {
    let last_synced = if cursor > 0 {
        cursor.to_string()
    } else {
        String::new()
    };
    let url = format!(
        "{}{}/sync/bp?nextpaginationKey=0&lastSyncedTime={last_synced}&phoneIdentifier=",
        session.base_url,
        path_prefix(&session.base_url)
    );
    let request = http
        .client
        .get(url)
        .header(USER_AGENT, APP_USER_AGENT)
        .header(AUTHORIZATION, session.access_token.as_str());
    let response: Response = send(request).await?;
    check_status(&response, &[401], Error::TokenExpired)?;

    let cap = http.max_reply_bytes.min(MAX_READINGS_REPLY_BYTES);
    let reply: ReadingsReply = read_json_streaming(response, cap).await?;

    Ok(reply.into_batch(cursor))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenReply {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
}

#[derive(Deserialize)]
struct ReadingsReply {
    data: Option<Vec<Item>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    user_number_in_device: Option<Num>,
    measurement_date: Option<Num>,
    time_zone: Option<Num>,
    systolic: Option<Num>,
    diastolic: Option<Num>,
    pulse: Option<Num>,
    #[serde(rename = "irregularHB")]
    irregular_hb: Option<Num>,
    movement_detect: Option<Num>,
    cuff_wrap_detect: Option<Num>,
    is_manual_entry: Option<Num>,
    device_model: Option<String>,
}

impl ReadingsReply {
    /// The reply carries no cursor, so the newest measurement time stands in
    /// for one. Overlap after a late upload is harmless: readings dedupe by key.
    fn into_batch(self, previous_cursor: i64) -> Batch {
        let mut batch = Batch {
            readings: Vec::new(),
            deleted: Vec::new(),
            cursor: previous_cursor,
        };

        for item in self.data.into_iter().flatten() {
            let Some(measured_at_ms) = item.measurement_date.and_then(Num::to_i64) else {
                continue;
            };

            batch.cursor = batch.cursor.max(measured_at_ms);

            if let Some(reading) = item.into_reading(measured_at_ms) {
                batch.readings.push(reading);
            }
        }

        batch
    }
}

impl Item {
    fn into_reading(self, measured_at_ms: i64) -> Option<Reading> {
        if nonzero(self.is_manual_entry) {
            return None;
        }

        let user_number = u32::try_from(self.user_number_in_device.and_then(Num::to_i64)?).ok()?;
        let to_u16 = |value: Option<Num>| u16::try_from(value.and_then(Num::to_i64)?).ok();
        let offset_seconds = self
            .time_zone
            .and_then(Num::to_i64)
            .and_then(|seconds| i32::try_from(seconds).ok());

        Some(Reading {
            key: reading_key("v2", user_number, measured_at_ms),
            measured_at_ms,
            zone: ReadingZone::OffsetSeconds(offset_seconds.unwrap_or(0)),
            systolic: to_u16(self.systolic)?,
            diastolic: to_u16(self.diastolic)?,
            pulse: to_u16(self.pulse).filter(|pulse| *pulse > 0),
            irregular_heartbeat: nonzero(self.irregular_hb),
            body_movement: nonzero(self.movement_detect),
            cuff_wrap_ok: self.cuff_wrap_detect.map(Num::is_nonzero),
            device_model: self.device_model.unwrap_or_default(),
            user_number,
        })
    }
}
