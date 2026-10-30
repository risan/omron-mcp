//! The v1 (Kii) cloud, which serves Indonesia and most of Asia and Latin
//! America. Verified against a live account.

use std::fmt;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{RequestBuilder, Response};
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::error::{Error, Result};
use crate::http::{Http, SMALL_REPLY_BYTES, check_status, read_json, read_json_streaming, send};
use crate::reading::{Batch, Reading, ReadingZone, reading_key};
use crate::region::V1Region;
use crate::session::{Session, Tokens};
use crate::wire::{Num, nonzero};

const APP_USER_AGENT: &str = "OmronConnect/011.004.00000.001 CFNetwork/1335.0.3.4 Darwin/21.6.0)";
const SDK_VERSION: &str = "000.005";
const APP_VERSION: &str = "011.004.00000";
const DEFAULT_EXPIRES_IN_SECS: i64 = 3600;
const CREDENTIAL_STATUSES: &[u16] = &[400, 401, 403];

fn post(http: &Http, region: V1Region, url: &str, body: serde_json::Value) -> RequestBuilder {
    http.client
        .post(url)
        .header(USER_AGENT, APP_USER_AGENT)
        .header("X-OGSC-SDK-Version", SDK_VERSION)
        .header("X-OGSC-App-Version", APP_VERSION)
        .header("X-Kii-AppID", region.app_id)
        .header("X-Kii-AppKey", region.app_key)
        .header(CONTENT_TYPE, "application/json")
        .body(body.to_string())
}

pub(crate) async fn login(
    http: &Http,
    region: V1Region,
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<Tokens> {
    let body = serde_json::json!({ "username": email, "password": password });
    let response = send(post(
        http,
        region,
        &format!("{base_url}/oauth2/token"),
        body,
    ))
    .await?;
    check_status(&response, CREDENTIAL_STATUSES, Error::BadCredentials)?;

    let reply: TokenReply = read_json(response, SMALL_REPLY_BYTES).await?;

    reply.into_tokens(None)
}

pub(crate) async fn refresh(http: &Http, region: V1Region, session: &Session) -> Result<Tokens> {
    let body = serde_json::json!({ "grant_type": "refresh_token", "refresh_token": session.refresh_token });
    let response = send(post(
        http,
        region,
        &format!("{}/oauth2/token", session.base_url),
        body,
    ))
    .await?;
    check_status(&response, CREDENTIAL_STATUSES, Error::TokenExpired)?;

    let reply: TokenReply = read_json(response, SMALL_REPLY_BYTES).await?;

    reply.into_tokens(Some(&session.refresh_token))
}

/// Asks the cloud whether anything is newer than `cursor`, without fetching it.
pub(crate) async fn has_new(
    http: &Http,
    region: V1Region,
    session: &Session,
    cursor: i64,
) -> Result<bool> {
    let response = sync_request(http, region, session, cursor, true).await?;
    let reply: SyncReply = read_json(response, SMALL_REPLY_BYTES).await?;
    let returned = reply.into_returned()?;

    Ok(returned
        .count_sync_data
        .and_then(Num::to_i64)
        .is_none_or(|count| count > 0))
}

pub(crate) async fn fetch(
    http: &Http,
    region: V1Region,
    session: &Session,
    cursor: i64,
) -> Result<Batch> {
    let response = sync_request(http, region, session, cursor, false).await?;
    let reply: SyncReply = read_json_streaming(response, http.max_reply_bytes).await?;

    reply.into_batch(cursor)
}

async fn sync_request(
    http: &Http,
    region: V1Region,
    session: &Session,
    cursor: i64,
    count_only: bool,
) -> Result<Response> {
    let url = format!(
        "{}/apps/{}/server-code/versions/current/synchronizeMeasureData",
        session.base_url, region.app_id
    );
    let body =
        serde_json::json!({ "lastSyncDate": cursor, "countOnlyFlag": i32::from(count_only) });
    let request = post(http, region, &url, body)
        .header(AUTHORIZATION, format!("Bearer {}", session.access_token));
    let response = send(request).await?;
    check_status(&response, &[401], Error::TokenExpired)?;

    Ok(response)
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    expires_in: Option<Num>,
}

impl TokenReply {
    fn into_tokens(self, previous_refresh_token: Option<&str>) -> Result<Tokens> {
        let refresh_token = match (self.refresh_token.is_empty(), previous_refresh_token) {
            (false, _) => self.refresh_token,
            (true, Some(previous)) => previous.to_owned(),
            (true, None) => String::new(),
        };

        if self.access_token.is_empty() || refresh_token.is_empty() {
            return Err(Error::BadReply("the sign-in reply has no tokens".into()));
        }

        let expires_in_secs = self
            .expires_in
            .and_then(Num::to_i64)
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_EXPIRES_IN_SECS);

        Ok(Tokens {
            access_token: self.access_token,
            refresh_token,
            expires_in_secs,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncReply {
    returned_value: Option<ReturnedValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReturnedValue {
    last_sync_date: Option<Num>,
    count_sync_data: Option<Num>,
    sync_list: Option<Vec<SyncItem>>,
    error_code: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncItem {
    device_category_list: Option<Vec<Category>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Category {
    device_model_list: Option<Vec<Model>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Model {
    device_model: Option<String>,
    #[serde(rename = "deviceSerialIDList")]
    device_serial_id_list: Option<Vec<Device>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    #[serde(rename = "deviceSerialID")]
    device_serial_id: Option<String>,
    user_number_in_device: Option<Num>,
    measure_list: Option<MeasureList>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Measure {
    time_zone: Option<String>,
    delete_flag: Option<Num>,
    measure_date_to: Option<Num>,
    body_index_list: BodyIndex,
}

impl Measure {
    fn is_deleted(&self) -> bool {
        nonzero(self.delete_flag)
    }

    fn is_blood_pressure(&self) -> bool {
        self.body_index_list.systolic.is_some() && self.body_index_list.diastolic.is_some()
    }
}

/// The measures worth keeping. Pulse-wave records (the same reading's raw
/// waveform) are dropped here, while the list is still being read, so they
/// never pile up.
struct MeasureList(Vec<Measure>);

impl<'de> Deserialize<'de> for MeasureList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ListVisitor;

        impl<'de> Visitor<'de> for ListVisitor {
            type Value = MeasureList;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a list of measures")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<MeasureList, A::Error> {
                let mut kept = Vec::new();

                while let Some(measure) = seq.next_element::<Measure>()? {
                    let deleted_reading =
                        measure.is_deleted() && !measure.body_index_list.pulse_wave;

                    if measure.is_blood_pressure() || deleted_reading {
                        kept.push(measure);
                    }
                }

                Ok(MeasureList(kept))
            }
        }

        deserializer.deserialize_seq(ListVisitor)
    }
}

/// The few body index codes a blood pressure reading needs, already scaled.
/// Every other code, and the `ppwData` pulse wave, is skipped without being
/// stored.
#[derive(Default)]
struct BodyIndex {
    systolic: Option<f64>,
    diastolic: Option<f64>,
    pulse: Option<f64>,
    irregular_heartbeat: Option<f64>,
    body_movement: Option<f64>,
    cuff_wrap: Option<f64>,
    pulse_wave: bool,
}

#[derive(Clone, Copy)]
enum Code {
    Systolic,
    Diastolic,
    Pulse,
    IrregularHeartbeat,
    BodyMovement,
    CuffWrap,
    PulseWave,
    Other,
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct CodeVisitor;

        impl Visitor<'_> for CodeVisitor {
            type Value = Code;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a body index code")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Code, E> {
                Ok(match value {
                    "1" => Code::Systolic,
                    "2" => Code::Diastolic,
                    "3" => Code::Pulse,
                    "6" => Code::IrregularHeartbeat,
                    "7" => Code::BodyMovement,
                    "8" => Code::CuffWrap,
                    "ppwData" => Code::PulseWave,
                    _ => Code::Other,
                })
            }
        }

        deserializer.deserialize_identifier(CodeVisitor)
    }
}

impl<'de> Deserialize<'de> for BodyIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct BodyVisitor;

        impl<'de> Visitor<'de> for BodyVisitor {
            type Value = BodyIndex;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a body index list")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<BodyIndex, A::Error> {
                let mut body = BodyIndex::default();

                while let Some(code) = map.next_key::<Code>()? {
                    let slot = match code {
                        Code::Systolic => &mut body.systolic,
                        Code::Diastolic => &mut body.diastolic,
                        Code::Pulse => &mut body.pulse,
                        Code::IrregularHeartbeat => &mut body.irregular_heartbeat,
                        Code::BodyMovement => &mut body.body_movement,
                        Code::CuffWrap => &mut body.cuff_wrap,
                        Code::PulseWave => {
                            body.pulse_wave = true;
                            map.next_value::<IgnoredAny>()?;
                            continue;
                        }
                        Code::Other => {
                            map.next_value::<IgnoredAny>()?;
                            continue;
                        }
                    };

                    *slot = map.next_value::<Figure>()?.0;
                }

                Ok(body)
            }
        }

        deserializer.deserialize_map(BodyVisitor)
    }
}

/// `[value, unit, scale, measurementId]`, all strings. The number is
/// `value x 10^scale`.
struct Figure(Option<f64>);

impl<'de> Deserialize<'de> for Figure {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct FigureVisitor;

        impl<'de> Visitor<'de> for FigureVisitor {
            type Value = Figure;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a [value, unit, scale, id] list")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Figure, A::Error> {
                let value = seq.next_element::<Option<Num>>()?.flatten();
                let _unit = seq.next_element::<IgnoredAny>()?;
                let scale = seq.next_element::<Option<Num>>()?.flatten();

                while seq.next_element::<IgnoredAny>()?.is_some() {}

                let scale = scale.map_or(0.0, |scale| scale.0);
                let scaled = value
                    .map(|value| value.0 * 10f64.powf(scale))
                    .filter(|number| number.is_finite());

                Ok(Figure(scaled))
            }
        }

        deserializer.deserialize_seq(FigureVisitor)
    }
}

impl SyncReply {
    fn into_returned(self) -> Result<ReturnedValue> {
        let returned = self
            .returned_value
            .ok_or_else(|| Error::BadReply("the sync reply has no returnedValue".into()))?;

        if returned.error_code.is_some() {
            return Err(Error::BadReply("the sync call reported an error".into()));
        }

        Ok(returned)
    }

    fn into_batch(self, previous_cursor: i64) -> Result<Batch> {
        let returned = self.into_returned()?;
        let cursor = returned
            .last_sync_date
            .and_then(Num::to_i64)
            .unwrap_or(previous_cursor);
        let mut batch = Batch {
            readings: Vec::new(),
            deleted: Vec::new(),
            cursor,
        };

        let devices = returned
            .sync_list
            .iter()
            .flatten()
            .flat_map(|item| item.device_category_list.iter().flatten())
            .flat_map(|category| category.device_model_list.iter().flatten())
            .flat_map(|model| {
                model
                    .device_serial_id_list
                    .iter()
                    .flatten()
                    .map(move |device| (model, device))
            });

        for (model, device) in devices {
            let (Some(serial), Some(user_number)) = (
                device.device_serial_id.as_deref(),
                device
                    .user_number_in_device
                    .and_then(Num::to_i64)
                    .and_then(|user| u32::try_from(user).ok()),
            ) else {
                continue;
            };

            let model_name = model.device_model.as_deref().unwrap_or_default();

            for measure in device.measure_list.iter().flat_map(|list| &list.0) {
                let Some(measured_at_ms) = measure.measure_date_to.and_then(Num::to_i64) else {
                    continue;
                };

                let key = reading_key(serial, user_number, measured_at_ms);

                if measure.is_deleted() {
                    batch.deleted.push(key);
                    continue;
                }

                if let Some(reading) =
                    build_reading(measure, key, measured_at_ms, model_name, user_number)
                {
                    batch.readings.push(reading);
                }
            }
        }

        Ok(batch)
    }
}

fn build_reading(
    measure: &Measure,
    key: String,
    measured_at_ms: i64,
    model: &str,
    user_number: u32,
) -> Option<Reading> {
    let body = &measure.body_index_list;
    let to_u16 =
        |figure: Option<f64>| figure.and_then(|number| u16::try_from(Num(number).to_i64()?).ok());

    Some(Reading {
        key,
        measured_at_ms,
        zone: ReadingZone::Named(measure.time_zone.clone().unwrap_or_else(|| "UTC".into())),
        systolic: to_u16(body.systolic)?,
        diastolic: to_u16(body.diastolic)?,
        pulse: to_u16(body.pulse),
        irregular_heartbeat: body.irregular_heartbeat.is_some_and(|flag| flag != 0.0),
        body_movement: body.body_movement.is_some_and(|flag| flag != 0.0),
        cuff_wrap_ok: body.cuff_wrap.map(|flag| flag != 0.0),
        device_model: model.to_owned(),
        user_number,
    })
}
