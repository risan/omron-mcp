//! The CLI commands driven in-process, including `serve` running next to them.

mod support;

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mcp_starter::cli::{Getenv, Io, Outcome};
use mcp_starter::store::{Change, Token, TokenKind};
use mcp_starter::test_support::Capture;
use omron_mcp::cli::{self, Settings};
use omron_mcp::store::OmronState;
use support::fake_omron::{FakeOmron, FakeReading};
use support::{DAY, EMAIL, PASSWORD, T0, open_store, wait_until};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const LOGIN_INPUT: &str = "id\nreader@example.test\nfixture-password\n";

struct Cli {
    fake: FakeOmron,
    dir: TempDir,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Serving {
    address: String,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<Outcome>,
}

impl Cli {
    async fn new() -> Cli {
        support::install_crypto_provider();

        let cli = Cli {
            fake: FakeOmron::start().await,
            dir: tempfile::tempdir().unwrap(),
        };

        cli.fake.add(FakeReading::new(T0, 132, 86, 71));
        cli.fake.add(FakeReading::new(T0 + DAY, 126, 82, 69));

        cli
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn getenv(&self, public_url: bool) -> Getenv {
        let data_dir = self.data_dir().display().to_string();

        Arc::new(move |key| match key {
            "OMRON_MCP_DATA_DIR" => data_dir.clone(),
            "OMRON_MCP_PUBLIC_URL" if public_url => "https://omron.example.com".to_string(),
            "OMRON_MCP_LISTEN" => "127.0.0.1:0".to_string(),
            _ => String::new(),
        })
    }

    fn settings(&self, shutdown: Option<CancellationToken>) -> Settings {
        Settings {
            base_url: Some(self.fake.base_url.clone()),
            shutdown,
            ..Settings::production()
        }
    }

    async fn run(&self, stdin: &str, args: &[&str]) -> Run {
        self.run_with(stdin, args, self.getenv(true)).await
    }

    async fn run_with(&self, stdin: &str, args: &[&str], getenv: Getenv) -> Run {
        let (stdout, stderr) = (Capture::default(), Capture::default());
        let io = Io {
            stdin: Box::new(Cursor::new(stdin.as_bytes().to_vec())),
            stdin_is_tty: false,
            stdout: Box::new(stdout.clone()),
            stderr: Box::new(stderr.clone()),
        };
        let args = args.iter().map(|arg| arg.to_string()).collect();
        let outcome = cli::run(args, io, getenv, self.settings(None)).await;

        Run {
            code: outcome.code,
            stdout: stdout.text(),
            stderr: stderr.text(),
        }
    }

    fn store(&self) -> mcp_starter::store::Store<OmronState> {
        open_store(&self.data_dir())
    }

    async fn state(&self) -> mcp_starter::store::State<OmronState> {
        self.store().read(|state| state.clone()).await.unwrap()
    }

    async fn start_serve(&self) -> Serving {
        let stderr = Capture::default();
        let io = Io {
            stdin: Box::new(Cursor::new(Vec::new())),
            stdin_is_tty: false,
            stdout: Box::new(Capture::default()),
            stderr: Box::new(stderr.clone()),
        };
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(cli::run(
            vec!["serve".to_string()],
            io,
            self.getenv(true),
            self.settings(Some(shutdown.clone())),
        ));

        wait_until(Duration::from_secs(10), "serve to listen", || {
            stderr.text().contains("listening on")
        })
        .await;

        let text = stderr.text();
        let address = text
            .split("listening on ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap()
            .to_string();

        Serving {
            address,
            shutdown,
            task,
        }
    }
}

impl Serving {
    async fn stop(self) -> Outcome {
        self.shutdown.cancel();

        self.task.await.unwrap()
    }
}

#[tokio::test]
async fn login_signs_in_with_a_piped_secret_and_runs_the_first_sync() {
    let cli = Cli::new().await;

    let run = cli.run(LOGIN_INPUT, &["login"]).await;
    let state = cli.state().await;
    let session = state.section.session.as_ref().unwrap();

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(session.email, EMAIL);
    assert_eq!(session.country, "ID", "the country is upper-cased");
    assert_eq!(state.section.login_rev, 1);
    assert!(state.section.cursor_ms > T0);
    assert!(
        run.stdout
            .contains("Signed in to OMRON connect as reader@example.test (ID)")
    );
    assert!(
        run.stdout.contains("Synced 2 new readings; 2 cached."),
        "{}",
        run.stdout
    );
    assert!(cli.data_dir().join("readings.json").is_file());

    let output = run.stdout + &run.stderr;

    for secret in [PASSWORD, "access-1", "refresh-1"] {
        assert!(!output.contains(secret), "{secret} was printed");
    }

    let state_file = std::fs::read_to_string(cli.data_dir().join("state.json")).unwrap();

    assert!(!state_file.contains(PASSWORD));
}

#[tokio::test]
async fn login_with_a_wrong_password_saves_nothing() {
    let cli = Cli::new().await;

    let run = cli
        .run("ID\nreader@example.test\nwrong-password\n", &["login"])
        .await;

    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("omron-mcp login: OMRON refused the email or password"),
        "{}",
        run.stderr
    );
    assert!(!(run.stdout.clone() + &run.stderr).contains("wrong-password"));
    assert!(cli.state().await.section.session.is_none());
    assert!(!cli.data_dir().join("readings.json").exists());
}

#[tokio::test]
async fn login_needs_all_three_answers() {
    let cli = Cli::new().await;

    for input in [
        "\nreader@example.test\nfixture-password\n",
        "ID\n\nfixture-password\n",
        "ID\nreader@example.test\n",
    ] {
        let run = cli.run(input, &["login"]).await;

        assert_eq!(run.code, 1, "{input:?}");
        assert!(run.stderr.contains("all required"), "{}", run.stderr);
    }

    assert!(cli.fake.calls().is_empty());
}

#[tokio::test]
async fn login_rejects_a_country_omron_does_not_serve() {
    let cli = Cli::new().await;

    let run = cli
        .run("ZZ\nreader@example.test\nfixture-password\n", &["login"])
        .await;

    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains("not available for country"),
        "{}",
        run.stderr
    );
}

#[tokio::test]
async fn a_failed_first_sync_still_keeps_the_login() {
    let cli = Cli::new().await;

    cli.fake.force_status(500);

    let run = cli.run(LOGIN_INPUT, &["login"]).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains("The first sync failed"),
        "{}",
        run.stdout
    );
    assert!(cli.state().await.section.session.is_some());
    assert_eq!(cli.state().await.section.cursor_ms, 0);
}

#[tokio::test]
async fn status_on_a_fresh_install() {
    let cli = Cli::new().await;

    let run = cli.run("", &["status"]).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("Login: not set (run: omron-mcp login)"));
    assert!(run.stdout.contains("Synced up to: never"));
    assert!(run.stdout.contains("Cached readings: 0"));
    assert!(run.stdout.contains("Newest reading: none"));
    assert!(run.stdout.contains("Owner password set: no"));
    assert!(cli.fake.calls().is_empty(), "status never calls OMRON");
}

#[tokio::test]
async fn status_after_login_shows_the_state_and_no_secrets() {
    let cli = Cli::new().await;

    cli.run(LOGIN_INPUT, &["login"]).await;
    cli.run(
        "a long enough password\na long enough password\n",
        &["set-password"],
    )
    .await;

    let calls = cli.fake.calls().len();
    let run = cli.run("", &["status"]).await;
    let state = cli.state().await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains("OMRON account: reader@example.test"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("Country: ID"));
    assert!(run.stdout.contains("Login: ok"));
    assert!(run.stdout.contains("Cached readings: 2"));
    assert!(
        run.stdout.contains("Newest reading: 2023-11-16 05:13 WIB"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("Owner password set: yes"));
    assert!(
        run.stdout.contains("Synced up to: 2023-11-"),
        "{}",
        run.stdout
    );

    let output = run.stdout + &run.stderr;
    let session = state.section.session.unwrap();

    for secret in [
        PASSWORD,
        &session.access_token,
        &session.refresh_token,
        &state.owner.password_hash,
        "a long enough password",
    ] {
        assert!(!output.contains(secret), "{secret} was printed");
    }

    assert_eq!(cli.fake.calls().len(), calls, "status never calls OMRON");
}

#[tokio::test]
async fn status_says_when_the_login_is_invalid() {
    let cli = Cli::new().await;

    cli.run(LOGIN_INPUT, &["login"]).await;
    cli.store()
        .update(|state| {
            state.section.login_invalid = true;

            Change::Write(())
        })
        .await
        .unwrap();

    let run = cli.run("", &["status"]).await;

    assert!(
        run.stdout
            .contains("Login: INVALID, OMRON refused it (run: omron-mcp login)")
    );
}

#[tokio::test]
async fn set_password_rules_and_revoke_tokens() {
    let cli = Cli::new().await;
    let expires_at = "2099-01-01T00:00:00Z".parse().unwrap();

    cli.store()
        .update(move |state| {
            state.tokens.insert(
                "h".to_string(),
                Token {
                    kind: TokenKind::Access,
                    client_id: String::new(),
                    family: String::new(),
                    scope: String::new(),
                    resource: String::new(),
                    expires_at,
                    rotated: false,
                    extra: Default::default(),
                },
            );

            Change::Write(())
        })
        .await
        .unwrap();

    let short = cli.run("short\nshort\n", &["set-password"]).await;

    assert!(
        short.code != 0 && short.stderr.contains("12"),
        "{}",
        short.stderr
    );

    let mismatch = cli
        .run(
            "a long enough password\nanother long enough one\n",
            &["set-password"],
        )
        .await;

    assert!(
        mismatch.code != 0 && mismatch.stderr.contains("match"),
        "{}",
        mismatch.stderr
    );
    assert!(cli.state().await.owner.password_hash.is_empty());

    let run = cli
        .run(
            "a long enough password\na long enough password\n",
            &["set-password"],
        )
        .await;
    let state = cli.state().await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(state.owner.password_hash.starts_with("$2"));
    assert!(state.tokens.is_empty(), "tokens revoked");
    assert!(!(run.stdout + &run.stderr).contains("a long enough password"));

    let revoke = cli.run("", &["revoke-tokens"]).await;

    assert_eq!(revoke.code, 0, "{}", revoke.stderr);
}

#[tokio::test]
async fn unknown_command_and_help() {
    let cli = Cli::new().await;

    let unknown = cli.run("", &["frobnicate"]).await;

    assert!(
        unknown.code == 2 && unknown.stderr.contains("usage"),
        "{}",
        unknown.stderr
    );

    for flag in ["help", "--help", "-h"] {
        let help = cli.run("", &[flag]).await;

        assert!(help.code == 0 && help.stdout.contains("login"), "{flag}");
    }
}

#[tokio::test]
async fn serve_refuses_corrupt_state() {
    let cli = Cli::new().await;

    std::fs::create_dir_all(cli.data_dir()).unwrap();

    let path = cli.data_dir().join("state.json");

    std::fs::write(&path, b"{broken").unwrap();

    let run = cli.run("", &["serve"]).await;

    assert!(
        run.code != 0 && run.stderr.contains(&path.display().to_string()),
        "{}",
        run.stderr
    );
}

#[tokio::test]
async fn serve_requires_public_url() {
    let cli = Cli::new().await;

    let run = cli.run_with("", &["serve"], cli.getenv(false)).await;

    assert!(
        run.code != 0 && run.stderr.contains("OMRON_MCP_PUBLIC_URL"),
        "{}",
        run.stderr
    );
}

#[tokio::test]
async fn serve_serves_the_home_page_refreshes_the_login_and_stops_on_shutdown() {
    let cli = Cli::new().await;

    cli.run(LOGIN_INPUT, &["login"]).await;

    let serving = cli.start_serve().await;
    let client = reqwest::Client::new();
    let home = client
        .get(format!("http://{}/", serving.address))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let health = client
        .get(format!("http://{}/healthz", serving.address))
        .send()
        .await
        .unwrap();

    assert!(home.contains("<title>omron-mcp: OMRON blood pressure for Claude and ChatGPT</title>"));
    assert!(health.status().is_success());

    wait_until(Duration::from_secs(10), "the start-up keep-alive", || {
        cli.fake.token_calls("refresh") == 1
    })
    .await;

    let outcome = serving.stop().await;

    assert_eq!(outcome.code, 0);
}

#[tokio::test]
async fn serve_refuses_a_second_instance() {
    let cli = Cli::new().await;
    let first = cli.start_serve().await;

    let second = cli.run("", &["serve"]).await;

    assert_ne!(second.code, 0);
    assert!(
        second
            .stderr
            .contains("another omron-mcp process is using this data dir"),
        "{}",
        second.stderr
    );

    drop(first.stop().await);
}
