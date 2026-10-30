use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The time zone a reading was taken in, as the cloud reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingZone {
    /// An IANA name such as `Asia/Jakarta` (v1).
    Named(String),
    /// A fixed offset from UTC in seconds (v2).
    OffsetSeconds(i32),
}

/// One blood pressure reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    /// Stable id for deduping and deletes. It hashes the device serial, the
    /// user number and the time, so the serial itself never leaves the
    /// library.
    pub key: String,
    /// Epoch milliseconds.
    pub measured_at_ms: i64,
    pub zone: ReadingZone,
    /// mmHg.
    pub systolic: u16,
    pub diastolic: u16,
    /// Beats per minute.
    pub pulse: Option<u16>,
    pub irregular_heartbeat: bool,
    pub body_movement: bool,
    /// Whether the cuff-wrap check passed. The cloud's code 8 is 1 on every
    /// normal reading seen, so a non-zero value is read as "wrapped well".
    /// That meaning is an educated guess, not documented by OMRON.
    pub cuff_wrap_ok: Option<bool>,
    pub device_model: String,
    /// The cuff's user slot (1 or 2 on a two-user cuff).
    pub user_number: u32,
}

/// What one sync call brought back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub readings: Vec<Reading>,
    /// Keys of readings the user deleted in the app. Drop them from any cache.
    pub deleted: Vec<String>,
    /// Pass this to the next call. It is the old cursor when the reply carried
    /// no new one.
    pub cursor: i64,
}

pub(crate) fn reading_key(serial: &str, user_number: u32, measured_at_ms: i64) -> String {
    let digest = Sha256::digest(format!("{serial}\u{0}{user_number}\u{0}{measured_at_ms}"));

    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
