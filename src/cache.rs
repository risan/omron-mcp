//! The readings cache, `<data_dir>/readings.json`: compact readings sorted by time, kept apart
//! from `state.json` so the state file stays small. A call loads it and drops it again, so the
//! resident memory does not grow with the history.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use mcp_starter::store::Store;
use omron_connect::Reading;
use serde::{Deserialize, Serialize};

use crate::store::OmronState;

const FILE_NAME: &str = "readings.json";

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    readings: Vec<Reading>,
}

/// What a merge did to the cache.
#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    pub added: usize,
    pub total: usize,
}

#[derive(Clone)]
pub struct ReadingCache {
    store: Store<OmronState>,
    path: PathBuf,
}

impl ReadingCache {
    pub fn new(store: &Store<OmronState>) -> ReadingCache {
        ReadingCache {
            path: store.data_dir().join(FILE_NAME),
            store: store.clone(),
        }
    }

    /// All cached readings, oldest first. A missing file is an empty cache.
    pub fn load(&self) -> Result<Vec<Reading>, String> {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("read {}: {error}", self.path.display())),
        };
        let file: CacheFile = serde_json::from_slice(&data)
            .map_err(|error| format!("parse {}: {error}", self.path.display()))?;

        Ok(file.readings)
    }

    /// Adds the new readings (a reading already cached under the same key is replaced), drops
    /// the deleted ones and writes the file again.
    pub fn merge(&self, fresh: Vec<Reading>, deleted: &[String]) -> Result<Merged, String> {
        let before = self.load()?;
        let known: HashSet<&str> = before.iter().map(|reading| reading.key.as_str()).collect();
        let added = fresh
            .iter()
            .filter(|reading| !known.contains(reading.key.as_str()))
            .map(|reading| reading.key.as_str())
            .collect::<HashSet<_>>()
            .len();
        let readings = merge_readings(before, fresh, deleted);
        let total = readings.len();

        self.write(&CacheFile { readings })?;

        Ok(Merged { added, total })
    }

    pub fn clear(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                Err(format!("remove {}: {error}", self.path.display()))
            }
            _ => Ok(()),
        }
    }

    fn write(&self, file: &CacheFile) -> Result<(), String> {
        let data = serde_json::to_vec(file).map_err(|error| format!("encode readings: {error}"))?;
        let temp = self
            .path
            .with_extension(format!("json.tmp-{}", unique_suffix()));
        let result = self.write_temp_and_rename(&temp, &data);

        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }

        result
    }

    fn write_temp_and_rename(&self, temp: &PathBuf, data: &[u8]) -> Result<(), String> {
        let describe = |what: &str, error: &dyn std::fmt::Display| {
            format!("{what} {}: {error}", temp.display())
        };
        let mut file: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temp)
            .map_err(|error| describe("create", &error))?;

        self.store
            .chown_to_dir_owner(temp)
            .map_err(|error| describe("chown", &error))?;
        file.write_all(data)
            .map_err(|error| describe("write", &error))?;
        file.sync_all().map_err(|error| describe("sync", &error))?;
        drop(file);

        fs::rename(temp, &self.path)
            .map_err(|error| format!("replace {}: {error}", self.path.display()))
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());

    format!("{}-{nanos}", process::id())
}

/// The old readings and the new ones by key, minus the deleted keys, oldest first.
pub fn merge_readings(
    existing: Vec<Reading>,
    fresh: Vec<Reading>,
    deleted: &[String],
) -> Vec<Reading> {
    let mut by_key: BTreeMap<String, Reading> = existing
        .into_iter()
        .chain(fresh)
        .map(|reading| (reading.key.clone(), reading))
        .collect();

    for key in deleted {
        by_key.remove(key);
    }

    let mut readings: Vec<Reading> = by_key.into_values().collect();

    readings.sort_by(|a, b| {
        a.measured_at_ms
            .cmp(&b.measured_at_ms)
            .then_with(|| a.key.cmp(&b.key))
    });

    readings
}

#[cfg(test)]
mod tests {
    use omron_connect::ReadingZone;

    use super::*;

    fn reading(key: &str, measured_at_ms: i64, systolic: u16) -> Reading {
        Reading {
            key: key.to_string(),
            measured_at_ms,
            zone: ReadingZone::Named("Asia/Jakarta".to_string()),
            systolic,
            diastolic: 80,
            pulse: Some(60),
            irregular_heartbeat: false,
            body_movement: false,
            cuff_wrap_ok: Some(true),
            device_model: "TEST-BP-1".to_string(),
            user_number: 1,
        }
    }

    fn keys(readings: &[Reading]) -> Vec<&str> {
        readings
            .iter()
            .map(|reading| reading.key.as_str())
            .collect()
    }

    #[test]
    fn merging_sorts_by_time_and_ignores_duplicate_keys() {
        let merged = merge_readings(
            vec![reading("b", 200, 120), reading("a", 100, 121)],
            vec![reading("c", 150, 122), reading("b", 200, 123)],
            &[],
        );

        assert_eq!(keys(&merged), ["a", "c", "b"]);
        assert_eq!(
            merged[2].systolic, 123,
            "a re-delivered reading replaces the old one"
        );
    }

    #[test]
    fn merging_drops_the_deleted_keys() {
        let merged = merge_readings(
            vec![reading("a", 100, 120), reading("b", 200, 121)],
            vec![reading("c", 300, 122)],
            &["b".to_string(), "unknown".to_string()],
        );

        assert_eq!(keys(&merged), ["a", "c"]);
    }

    #[test]
    fn readings_with_the_same_time_keep_a_stable_order() {
        let merged = merge_readings(
            vec![],
            vec![reading("z", 100, 1), reading("y", 100, 2)],
            &[],
        );

        assert_eq!(keys(&merged), ["y", "z"]);
    }
}
