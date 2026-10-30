//! The settings omron-mcp hands to mcp-starter's config: the variable names and the defaults.

use std::collections::HashMap;
use std::path::PathBuf;

use mcp_starter::config::Config;
use omron_mcp::SERVICE;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> String {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();

    move |key| map.get(key).cloned().unwrap_or_default()
}

#[test]
fn defaults() {
    let config = Config::load(&SERVICE, &env(&[]), false).unwrap();

    assert_eq!(config.public_url, "");
    assert_eq!(config.listen, "127.0.0.1:8082");
    assert_eq!(config.data_dir, PathBuf::from("/var/lib/omron-mcp"));
    assert_eq!(config.tz.iana_name(), Some("Asia/Jakarta"));
    assert_eq!(SERVICE.scope, "omron");
}

#[test]
fn overrides_are_read() {
    let config = Config::load(
        &SERVICE,
        &env(&[
            ("OMRON_MCP_PUBLIC_URL", "https://omron.example.com"),
            ("OMRON_MCP_LISTEN", "127.0.0.1:9000"),
            ("OMRON_MCP_DATA_DIR", "/tmp/data"),
            ("OMRON_MCP_TZ", "Europe/Amsterdam"),
        ]),
        true,
    )
    .unwrap();

    assert_eq!(config.public_url, "https://omron.example.com");
    assert_eq!(config.listen, "127.0.0.1:9000");
    assert_eq!(config.data_dir, PathBuf::from("/tmp/data"));
    assert_eq!(config.tz.iana_name(), Some("Europe/Amsterdam"));
}

#[test]
fn empty_public_url_is_an_error_only_for_serve() {
    assert!(Config::load(&SERVICE, &env(&[]), false).is_ok());

    let error = Config::load(&SERVICE, &env(&[]), true).err().unwrap();

    assert!(
        error
            .to_string()
            .contains("OMRON_MCP_PUBLIC_URL is required")
    );
}
