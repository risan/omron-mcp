//! The part of the state file that belongs to OMRON.

use jiff::Timestamp;
use mcp_starter::store::Section;
use omron_connect::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct OmronState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
    /// The `lastSyncDate` of the newest sync; 0 means nothing was synced yet.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cursor_ms: i64,
    /// Increments on every login so a running server notices a new session.
    #[serde(default)]
    pub login_rev: i64,
    /// OMRON refused the refresh token: only `omron-mcp login` can fix it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub login_invalid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<Timestamp>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Section for OmronState {
    const KEY: &'static str = "omron";
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}
