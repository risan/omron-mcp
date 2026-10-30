//! `tools/list` answers with the one tool, byte for byte: name, description, annotations and
//! input schema. Regenerate the fixture with `UPDATE_SNAPSHOT=1 cargo test --test
//! tools_list_snapshot` after an intended change.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::ToolsEnv;

#[tokio::test]
async fn tools_list_matches_the_snapshot() {
    let rig = ToolsEnv::start().await;
    let tools = Value::Array(rig.mcp.list_tools().await);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tools-list.json");

    if std::env::var_os("UPDATE_SNAPSHOT").is_some() {
        fs::write(&path, serde_json::to_string_pretty(&tools).unwrap() + "\n").unwrap();
    }

    let snapshot: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

    assert_eq!(snapshot.as_array().unwrap().len(), 1);
    assert_eq!(tools, snapshot);
}

#[tokio::test]
async fn the_tool_list_stays_small() {
    let rig = ToolsEnv::start().await;
    let tools = Value::Array(rig.mcp.list_tools().await);

    assert!(
        tools.to_string().len() < 3_000,
        "{}",
        tools.to_string().len()
    );
}
