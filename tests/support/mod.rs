//! Shared helpers for the integration tests. Every value here is invented.

#![allow(dead_code, unused_imports)]

pub mod fake_omron;
pub mod fake_v2;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mcp_starter::config::Config;
use mcp_starter::store::{Store, StoreOptions};
use mcp_starter::test_support::jsonrpc::{McpClient, ToolResult};
use mcp_starter::test_support::serve_mcp;
pub use mcp_starter::test_support::{TestClock, install_crypto_provider, wait_until};
use mcp_starter::work::Work;
use omron_connect::{Client, Reading};
use omron_mcp::SERVICE;
use omron_mcp::app::App;
use omron_mcp::cache::ReadingCache;
use omron_mcp::store::OmronState;
use omron_mcp::sync::{KEEP_ALIVE_EVERY, Omron};
use omron_mcp::tools::{OmronTools, ToolOptions};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_util::task::TaskTracker;

use fake_omron::FakeOmron;

pub const EMAIL: &str = "reader@example.test";
pub const PASSWORD: &str = "fixture-password";
pub const COUNTRY: &str = "ID";

/// 2023-11-14T22:13:20Z, which is 2023-11-15 05:13:20 in Jakarta.
pub const T0: i64 = 1_700_000_000_000;
pub const MINUTE: i64 = 60_000;
pub const DAY: i64 = 24 * 60 * MINUTE;
/// 2023-11-20 10:00 in Jakarta: the "today" of the tool tests.
pub const NOW: &str = "2023-11-20T03:00:00Z";

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
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
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

pub fn omron_client(base_url: &str) -> Client {
    install_crypto_provider();

    let http = mcp_starter::http::client_builder(None)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    Client::new(http).with_base_urls([base_url])
}

pub fn open_store(dir: &std::path::Path) -> Store<OmronState> {
    Store::open(dir, StoreOptions::default()).unwrap()
}

/// The fake OMRON, a data dir, and the sync service pointed at the fake.
pub struct Env {
    pub fake: FakeOmron,
    pub dir: TempDir,
    pub store: Store<OmronState>,
    pub omron: Omron,
}

impl Env {
    pub async fn start() -> Env {
        let fake = FakeOmron::start().await;
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path());
        let omron = Omron::new(
            omron_client(&fake.base_url),
            store.clone(),
            KEEP_ALIVE_EVERY,
        );

        Env {
            fake,
            dir,
            store,
            omron,
        }
    }

    pub async fn signed_in() -> Env {
        let env = Env::start().await;

        env.omron.sign_in(COUNTRY, EMAIL, PASSWORD).await.unwrap();

        env
    }

    pub async fn section(&self) -> OmronState {
        self.store
            .read(|state| state.section.clone())
            .await
            .unwrap()
    }

    pub fn cached(&self) -> Vec<Reading> {
        ReadingCache::new(&self.store).load().unwrap()
    }

    pub fn cache_path(&self) -> PathBuf {
        self.dir.path().join("readings.json")
    }
}

pub struct ToolsSetup {
    pub signed_in: bool,
    pub tool_deadline: Duration,
    pub sync_wait: Duration,
}

impl Default for ToolsSetup {
    fn default() -> ToolsSetup {
        ToolsSetup {
            signed_in: true,
            tool_deadline: Duration::from_secs(10),
            sync_wait: Duration::from_secs(8),
        }
    }
}

/// The MCP endpoint over real HTTP on a bare test router, with the tool talking to the fake.
pub struct ToolsEnv {
    pub env: Env,
    pub mcp: McpClient,
    pub clock: TestClock,
}

impl ToolsEnv {
    pub async fn start() -> ToolsEnv {
        ToolsEnv::start_with(ToolsSetup::default()).await
    }

    pub async fn start_with(setup: ToolsSetup) -> ToolsEnv {
        let env = if setup.signed_in {
            Env::signed_in().await
        } else {
            Env::start().await
        };
        let clock = TestClock::new(NOW);
        let tools = ToolOptions {
            now: clock.as_clock(),
            tool_deadline: setup.tool_deadline,
            sync_wait: setup.sync_wait,
        };
        let config = Config::load(&SERVICE, &|_| String::new(), false).unwrap();
        let work = Work::new(TaskTracker::new());
        let app = Arc::new(App::new(env.omron.clone(), config, tools, work));
        let mcp = serve_mcp(OmronTools::factory(app)).await;

        ToolsEnv { env, mcp, clock }
    }

    pub fn fake(&self) -> &FakeOmron {
        &self.env.fake
    }

    pub async fn blood_pressure(&self, arguments: Value) -> ToolResult {
        self.mcp.call("omron_get_blood_pressure", arguments).await
    }
}

pub fn rows(result: &ToolResult) -> Vec<Value> {
    result.structured["readings"].as_array().unwrap().clone()
}
