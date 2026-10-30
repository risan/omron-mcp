//! The `omron` section of `state.json` loads and saves back unchanged, unknown keys at every
//! level included, and a state file with no section loads as an empty one.

mod support;

use std::fs;

use mcp_starter::store::Change;
use serde_json::{Value, json};
use support::open_store;

fn state_file() -> Value {
    json!({
        "omron": {
            "session": {
                "country": "ID",
                "email": "reader@example.test",
                "base_url": "https://omron.example.test/api",
                "access_token": "fixture-access",
                "access_expires_at_ms": 4_102_444_800_000_i64,
                "refresh_token": "fixture-refresh"
            },
            "cursor_ms": 1_700_000_000_000_i64,
            "login_rev": 3,
            "login_invalid": true,
            "last_refresh_at": "2026-10-30T10:00:00Z",
            "added_later": {"nested": [1, 2]}
        },
        "owner": {"password_hash": "$2b$12$fixture"},
        "clients": {},
        "tokens": {},
        "unknown_top_level": true
    })
}

#[tokio::test]
async fn the_omron_section_saves_back_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");

    fs::write(&path, serde_json::to_vec(&state_file()).unwrap()).unwrap();
    fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();

    let store = open_store(dir.path());

    store.update(|_| Change::Write(())).await.unwrap();

    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

    assert_eq!(saved, state_file());

    let section = store.read(|state| state.section.clone()).await.unwrap();

    assert_eq!(section.cursor_ms, 1_700_000_000_000);
    assert_eq!(section.login_rev, 3);
    assert!(section.login_invalid);
    assert_eq!(section.session.unwrap().refresh_token, "fixture-refresh");
}

#[tokio::test]
async fn an_empty_section_writes_only_the_revision() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    store.update(|_| Change::Write(())).await.unwrap();

    let saved: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();

    assert_eq!(saved["omron"], json!({"login_rev": 0}));
}
