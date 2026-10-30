//! The one MCP tool, `omron_get_blood_pressure`. It syncs with OMRON in a tracked task of its
//! own, so a sync that outlasts the reply keeps running and the reply carries what the cache
//! already holds.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan, Zoned};
use mcp_starter::tools::{
    ToolOutcome, call_tool, date_or_today, error_result, request_permit, run_tool,
};
use mcp_starter::work::ConnPermit;
use mcp_starter::{Clock, system_clock};
use omron_connect::{Reading, ReadingZone};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use rmcp::{tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::App;
use crate::sync::SyncError;

const DEFAULT_DAYS: i64 = 30;
const MAX_DAYS: i64 = 366;

/// The clock and the durations of one tool call; tests scale them down.
#[derive(Clone)]
pub struct ToolOptions {
    pub now: Clock,
    /// The latest a tool call replies (default 40 s).
    pub tool_deadline: Duration,
    /// How long a call waits for the sync before it answers from the cache (default 30 s).
    pub sync_wait: Duration,
}

impl Default for ToolOptions {
    fn default() -> ToolOptions {
        ToolOptions {
            now: system_clock(),
            tool_deadline: Duration::from_secs(40),
            sync_wait: Duration::from_secs(30),
        }
    }
}

const DESCRIPTION_GET_BLOOD_PRESSURE: &str = "Read the owner's OMRON blood pressure readings, syncing with OMRON connect first. Dates are YYYY-MM-DD, taken in each reading's own time zone; the default is the 30 days ending today, at most 366 days. `user` picks one cuff user slot (1 or 2). Returns readings newest first (date, local time with seconds, utc_offset, systolic and diastolic in mmHg, pulse in bpm, irregular_heartbeat, body_movement, cuff_wrap_ok, device, user) and a summary with the count and the average, min and max. A note says when the sync failed or is still running and the readings come from the cache.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBloodPressureInput {
    /// First day, YYYY-MM-DD (default 29 days before end_date)
    pub start_date: Option<String>,
    /// Last day, YYYY-MM-DD (default today in the owner's timezone)
    pub end_date: Option<String>,
    /// Cuff user slot, such as 1 or 2 (default both)
    pub user: Option<u32>,
}

#[derive(Clone)]
pub struct OmronTools {
    app: Arc<App>,
    router: Arc<ToolRouter<OmronTools>>,
    /// The connection slot of the request being served, so a tool task keeps it until the
    /// task ends.
    permit: ConnPermit,
}

impl OmronTools {
    /// The factory `mcp_service` takes. The tool router (and its schemas) is built once.
    pub fn factory(app: Arc<App>) -> impl Fn() -> OmronTools + Send + Sync + 'static {
        let router = Arc::new(OmronTools::tool_router());

        move || OmronTools {
            app: app.clone(),
            router: router.clone(),
            permit: ConnPermit::default(),
        }
    }

    fn date_range(&self, start: Option<&str>, end: Option<&str>) -> Result<(Date, Date), String> {
        let now = (self.app.tools.now)();
        let end = date_or_today(&self.app.config, now, end)?;
        let start = match start.filter(|start| !start.is_empty()) {
            Some(_) => date_or_today(&self.app.config, now, start)?,
            None => end
                .checked_sub((DEFAULT_DAYS - 1).days())
                .map_err(|error| error.to_string())?,
        };

        if start > end {
            return Err(format!(
                "start_date ({start}) must not be after end_date ({end})"
            ));
        }

        let days = i64::from((end - start).get_days()) + 1;

        if days > MAX_DAYS {
            return Err(format!(
                "the range {start} to {end} is {days} days; at most {MAX_DAYS} days at once"
            ));
        }

        Ok((start, end))
    }

    /// Runs `body` as detached, tracked work and waits for it until the tool deadline.
    async fn run<F, Fut>(&self, tool: &str, body: F) -> CallToolResult
    where
        F: FnOnce(Arc<App>) -> Fut,
        Fut: Future<Output = ToolOutcome> + Send + 'static,
    {
        run_tool(
            &self.app.work,
            self.permit.clone(),
            self.app.tools.tool_deadline,
            tool,
            "",
            body(self.app.clone()),
        )
        .await
    }
}

#[tool_router]
impl OmronTools {
    #[tool(
        name = "omron_get_blood_pressure",
        description = DESCRIPTION_GET_BLOOD_PRESSURE,
        annotations(title = "Get blood pressure", read_only_hint = true, idempotent_hint = true)
    )]
    async fn get_blood_pressure(
        &self,
        Parameters(input): Parameters<GetBloodPressureInput>,
    ) -> CallToolResult {
        let (start, end) =
            match self.date_range(input.start_date.as_deref(), input.end_date.as_deref()) {
                Ok(range) => range,
                Err(message) => return error_result(message),
            };

        self.run("omron_get_blood_pressure", move |app| async move {
            let note = sync_note(&app).await?;
            let readings = app.omron.load_readings().await.map_err(text)?;

            Ok(present(&readings, start, end, input.user, note))
        })
        .await
    }
}

#[tool_handler(router = self.router, name = "omron-mcp")]
impl ServerHandler for OmronTools {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tools = OmronTools {
            permit: request_permit(&context),
            ..self.clone()
        };

        call_tool(&self.router, &tools, request, context).await
    }
}

/// Syncs in a task of its own and waits for it for `sync_wait`. `Ok(None)` is a finished
/// sync; the note says what went wrong or that the sync is still running.
async fn sync_note(app: &App) -> Result<Option<String>, String> {
    let omron = app.omron.clone();
    let handle = app
        .work
        .spawn(async move { omron.sync().await })
        .map_err(text)?;

    Ok(match tokio::time::timeout(app.tools.sync_wait, handle).await {
        Ok(Ok(Ok(_))) => None,
        Ok(Ok(Err(error))) => Some(failure_note(&error)),
        Ok(Err(_)) => Some("The sync stopped unexpectedly. These are the readings cached earlier.".to_string()),
        Err(_) => Some(
            "The sync with OMRON is still running (a first sync can take minutes). These are the readings cached so far; ask again shortly."
                .to_string(),
        ),
    })
}

fn failure_note(error: &SyncError) -> String {
    if error.needs_login() {
        return format!("{error}. These are the readings cached earlier.");
    }

    format!("Could not sync with OMRON: {error}. These are the readings cached earlier.")
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The readings of the range, newest first, with their summary.
pub fn present(
    readings: &[Reading],
    start: Date,
    end: Date,
    user: Option<u32>,
    note: Option<String>,
) -> (String, Value) {
    let mut chosen: Vec<(Zoned, &Reading)> = readings
        .iter()
        .filter(|reading| user.is_none_or(|user| reading.user_number == user))
        .map(|reading| (local_time(reading), reading))
        .filter(|(local, _)| (start..=end).contains(&local.date()))
        .collect();

    chosen.sort_by_key(|(_, reading)| std::cmp::Reverse(reading.measured_at_ms));

    let rows: Vec<Value> = chosen
        .iter()
        .map(|(local, reading)| {
            json!({
                "date": local.date().to_string(),
                "time": local.strftime("%H:%M:%S").to_string(),
                "utc_offset": local.strftime("%:z").to_string(),
                "systolic": reading.systolic,
                "diastolic": reading.diastolic,
                "pulse": reading.pulse,
                "irregular_heartbeat": reading.irregular_heartbeat,
                "body_movement": reading.body_movement,
                "cuff_wrap_ok": reading.cuff_wrap_ok,
                "device": reading.device_model,
                "user": reading.user_number,
            })
        })
        .collect();
    let picked: Vec<&Reading> = chosen.iter().map(|(_, reading)| *reading).collect();
    let summary = summarize(&picked);
    let mut headline = match (
        summary["systolic"]["average"].as_f64(),
        summary["diastolic"]["average"].as_f64(),
    ) {
        (Some(systolic), Some(diastolic)) => format!(
            "{} readings from {start} to {end}: average {systolic:.0}/{diastolic:.0} mmHg.",
            picked.len()
        ),
        _ => format!("No readings from {start} to {end}."),
    };
    let mut output = json!({
        "start_date": start.to_string(),
        "end_date": end.to_string(),
        "summary": summary,
        "readings": rows,
    });

    if let Some(note) = note {
        headline = format!("{headline} {note}");
        output["note"] = Value::String(note);
    }

    (headline, output)
}

fn local_time(reading: &Reading) -> Zoned {
    let zone = match &reading.zone {
        ReadingZone::Named(name) => TimeZone::get(name).unwrap_or(TimeZone::UTC),
        ReadingZone::OffsetSeconds(seconds) => {
            Offset::from_seconds(*seconds).map_or(TimeZone::UTC, TimeZone::fixed)
        }
    };

    Timestamp::from_millisecond(reading.measured_at_ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(zone)
}

fn summarize(readings: &[&Reading]) -> Value {
    if readings.is_empty() {
        return json!({"count": 0});
    }

    let pulses: Vec<u16> = readings
        .iter()
        .filter_map(|reading| reading.pulse)
        .collect();
    let mut summary = json!({
        "count": readings.len(),
        "systolic": stats(readings.iter().map(|reading| reading.systolic)),
        "diastolic": stats(readings.iter().map(|reading| reading.diastolic)),
        "irregular_heartbeat_count": readings.iter().filter(|reading| reading.irregular_heartbeat).count(),
    });

    if !pulses.is_empty() {
        summary["pulse"] = stats(pulses.into_iter());
    }

    summary
}

fn stats(values: impl Iterator<Item = u16> + Clone) -> Value {
    let count = values.clone().count();
    let sum: u64 = values.clone().map(u64::from).sum();
    let average = sum as f64 / count as f64;

    json!({
        "average": (average * 10.0).round() / 10.0,
        "min": values.clone().min(),
        "max": values.max(),
    })
}
