//! The command line: `serve` and the setup commands. `run` takes everything it reads from the
//! outside as arguments, so tests drive it in-process.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use mcp_starter::cli::{self as shared, Getenv, Io, Outcome, Serving};
use mcp_starter::config::Config;
use mcp_starter::store::{Lease, Store, StoreOptions};
use omron_connect::Client;
use tokio_util::sync::CancellationToken;

use crate::SERVICE;
use crate::app::App;
use crate::cache::ReadingCache;
use crate::store::OmronState;
use crate::sync::{KEEP_ALIVE_EVERY, Omron};
use crate::tools::{OmronTools, ToolOptions};

const USAGE: &str = "usage: omron-mcp [command]

commands:
  serve          run the MCP server (default)
  login          sign in to OMRON connect and run the first sync
  status         show the login, sync and password state (no secrets)
  set-password   set the owner password for the consent page
  revoke-tokens  delete all OAuth tokens and registered clients
";
const LOGIN_BUDGET: Duration = Duration::from_secs(60);
/// A first sync of years of readings is one long download.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Settings {
    /// Replaces the OMRON API root from the region table; tests point it at a fake.
    pub base_url: Option<String>,
    /// Ends `serve`. Without it, `serve` waits for SIGINT or SIGTERM.
    pub shutdown: Option<CancellationToken>,
    pub keep_alive_every: Duration,
    pub tools: ToolOptions,
}

impl Settings {
    pub fn production() -> Settings {
        Settings {
            base_url: None,
            shutdown: None,
            keep_alive_every: KEEP_ALIVE_EVERY,
            tools: ToolOptions::default(),
        }
    }
}

pub async fn run(args: Vec<String>, mut io: Io, getenv: Getenv, settings: Settings) -> Outcome {
    let command = args.first().map_or("serve", String::as_str);

    if matches!(command, "help" | "-h" | "--help") {
        let _ = write!(io.stdout, "{USAGE}");

        return Outcome::code(0);
    }

    if !matches!(
        command,
        "serve" | "login" | "status" | "set-password" | "revoke-tokens"
    ) {
        let _ = write!(io.stderr, "unknown command {command:?}\n{USAGE}");

        return Outcome::code(2);
    }

    let result = dispatch(command, &mut io, getenv, settings).await;

    match result {
        Ok(lease) => Outcome { code: 0, lease },
        Err(message) => {
            let _ = writeln!(io.stderr, "omron-mcp {command}: {message}");

            Outcome::code(1)
        }
    }
}

async fn dispatch(
    command: &str,
    io: &mut Io,
    getenv: Getenv,
    settings: Settings,
) -> Result<Option<Lease>, String> {
    #[cfg(feature = "testhooks")]
    let settings = testhooks::override_endpoint(settings, &getenv);
    let config = Config::load(&SERVICE, &*getenv, command == "serve").map_err(text)?;
    let store = Store::open(&config.data_dir, StoreOptions::default()).map_err(text)?;

    match command {
        "serve" => serve(io, config, store, settings).await.map(Some),
        "login" => login(io, store, settings).await.map(|()| None),
        "set-password" => shared::set_password(io, &store).await.map(|()| None),
        "status" => status(io, &config, &store).await.map(|()| None),
        _ => shared::revoke_tokens(io, &store).await.map(|()| None),
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Hooks the load test needs; compiled only with the `testhooks` feature, so the production
/// binary carries neither the code nor the variable name.
#[cfg(feature = "testhooks")]
mod testhooks {
    use super::*;

    pub fn override_endpoint(mut settings: Settings, getenv: &Getenv) -> Settings {
        let base = getenv("OMRON_MCP_TEST_BASE_URL");

        if !base.is_empty() {
            settings.base_url = Some(base);
        }

        settings
    }
}

fn omron_client(settings: &Settings) -> Client {
    let http = mcp_starter::http::client_builder(None)
        .timeout(REQUEST_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("the client settings are valid");
    let client = Client::new(http);

    match &settings.base_url {
        Some(url) => client.with_base_urls([url.as_str()]),
        None => client,
    }
}

async fn serve(
    io: &mut Io,
    config: Config,
    store: Store<OmronState>,
    settings: Settings,
) -> Result<Lease, String> {
    let serving = Serving::start(&SERVICE, config, store, settings.shutdown.clone()).await?;
    let tracker = serving.work.tracker().clone();
    let omron = Omron::new(
        omron_client(&settings),
        serving.store.clone(),
        settings.keep_alive_every,
    );
    let app = Arc::new(App::new(
        omron.clone(),
        serving.config.clone(),
        settings.tools.clone(),
        serving.work.clone(),
    ));
    let shutdown = serving.shutdown.clone();

    tracker.spawn(async move { omron.run_keep_alive(shutdown).await });

    serving.run(io, OmronTools::factory(app)).await
}

fn prompt_line(io: &mut Io, prompt: &str) -> Result<String, String> {
    let _ = write!(io.stderr, "{prompt}");
    let _ = io.stderr.flush();

    let mut line = String::new();

    io.stdin
        .read_line(&mut line)
        .map_err(|error| format!("read input: {error}"))?;

    Ok(line.trim().to_string())
}

async fn login(io: &mut Io, store: Store<OmronState>, settings: Settings) -> Result<(), String> {
    let country = prompt_line(io, "Country code (ISO 3166-1, for example ID): ")?;
    let email = prompt_line(io, "OMRON connect email: ")?;
    let password = shared::read_secret(io, "OMRON connect password (input hidden): ")?;

    if country.is_empty() || email.is_empty() || password.is_empty() {
        return Err("country, email and password are all required".to_string());
    }

    let omron = Omron::new(omron_client(&settings), store, settings.keep_alive_every);

    tokio::time::timeout(LOGIN_BUDGET, omron.sign_in(&country, &email, &password))
        .await
        .map_err(|_| "OMRON did not answer in time".to_string())?
        .map_err(text)?;

    let _ = writeln!(
        io.stdout,
        "Signed in to OMRON connect as {email} ({}).",
        country.to_ascii_uppercase()
    );
    let _ = writeln!(
        io.stderr,
        "Syncing your readings (a first sync can take a few minutes)..."
    );

    match omron.sync().await {
        Ok(added) => {
            let total = omron.load_readings().await.map_err(text)?.len();

            let _ = writeln!(io.stdout, "Synced {added} new readings; {total} cached.");
        }
        Err(error) => {
            let _ = writeln!(
                io.stdout,
                "The first sync failed ({error}). The server tries again on the next request."
            );
        }
    }

    Ok(())
}

async fn status(io: &mut Io, config: &Config, store: &Store<OmronState>) -> Result<(), String> {
    let now = Timestamp::now();
    let (report, oauth) = store
        .read(move |state| {
            let omron = &state.section;
            let time = |at: Option<Timestamp>| {
                at.map_or("never".to_string(), |at| {
                    at.strftime("%Y-%m-%d %H:%M:%S UTC").to_string()
                })
            };
            let login = match &omron.session {
                None => "not set (run: omron-mcp login)",
                Some(_) if omron.login_invalid => {
                    "INVALID, OMRON refused it (run: omron-mcp login)"
                }
                Some(_) => "ok",
            };
            let (account, country) = omron.session.as_ref().map_or_else(
                || ("unknown".to_string(), "unknown".to_string()),
                |session| (session.email.clone(), session.country.clone()),
            );
            let synced = Timestamp::from_millisecond(omron.cursor_ms)
                .ok()
                .filter(|_| omron.cursor_ms > 0);

            (
                format!(
                    "OMRON account: {account}\n\
                     Country: {country}\n\
                     Login: {login}\n\
                     Last token refresh: {}\n\
                     Synced up to: {}\n",
                    time(omron.last_refresh_at),
                    time(synced),
                ),
                shared::oauth_status(state, now),
            )
        })
        .await
        .map_err(text)?;
    let readings = ReadingCache::new(store).load()?;
    let newest = readings
        .iter()
        .map(|reading| reading.measured_at_ms)
        .max()
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .map_or("none".to_string(), |at| {
            at.to_zoned(config.tz.clone())
                .strftime("%Y-%m-%d %H:%M %Z")
                .to_string()
        });

    let _ = write!(
        io.stdout,
        "{report}Cached readings: {}\nNewest reading: {newest}\n{oauth}",
        readings.len()
    );

    Ok(())
}
