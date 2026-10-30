//! The sync service on its own: sign-in, the saved login, the keep-alive refresh.

mod support;

use std::time::Duration;

use omron_mcp::sync::{Omron, SyncError};
use support::fake_omron::FakeReading;
use support::{COUNTRY, DAY, EMAIL, Env, PASSWORD, T0, omron_client, wait_until};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn sign_in_saves_the_session_and_never_the_password() {
    let env = Env::start().await;

    env.omron.sign_in(COUNTRY, EMAIL, PASSWORD).await.unwrap();

    let section = env.section().await;
    let session = section.session.as_ref().unwrap();

    assert_eq!(session.email, EMAIL);
    assert_eq!(session.country, COUNTRY);
    assert_eq!(session.access_token, "access-1");
    assert_eq!(section.login_rev, 1);
    assert!(!section.login_invalid);

    let state_file = std::fs::read_to_string(env.store.path()).unwrap();

    assert!(!state_file.contains(PASSWORD));
    assert!(state_file.contains("\"omron\""));
    assert!(
        !format!("{section:?}").contains("access-1"),
        "Debug hides tokens"
    );
}

#[tokio::test]
async fn bad_credentials_save_nothing() {
    let env = Env::start().await;
    let error = env
        .omron
        .sign_in(COUNTRY, EMAIL, "wrong-password")
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        SyncError::Omron(omron_connect::Error::BadCredentials)
    ));
    assert!(env.section().await.session.is_none());
}

#[tokio::test]
async fn signing_in_again_to_the_same_account_keeps_the_cache_and_clears_the_invalid_flag() {
    let env = Env::signed_in().await;

    env.fake.add(FakeReading::new(T0, 130, 85, 70));
    env.omron.sync().await.unwrap();
    env.fake.expire_access_tokens();
    env.fake.refuse_refresh();

    env.fake.add(FakeReading::new(T0 + DAY, 131, 86, 71));

    assert!(matches!(
        env.omron.sync().await,
        Err(SyncError::LoginInvalid)
    ));
    assert!(env.section().await.login_invalid);

    let cursor = env.section().await.cursor_ms;
    env.fake.accept_refresh();
    env.omron
        .sign_in(COUNTRY, "READER@example.test", PASSWORD)
        .await
        .unwrap();

    let section = env.section().await;

    assert!(!section.login_invalid);
    assert_eq!(section.login_rev, 2);
    assert_eq!(section.cursor_ms, cursor);
    assert_eq!(env.cached().len(), 1);
}

#[tokio::test]
async fn signing_in_to_another_account_starts_with_an_empty_cache() {
    let env = Env::signed_in().await;

    env.fake.add(FakeReading::new(T0, 130, 85, 70));
    env.omron.sync().await.unwrap();

    assert_eq!(env.cached().len(), 1);
    assert!(env.section().await.cursor_ms > 0);

    env.omron.sign_in("MY", EMAIL, PASSWORD).await.unwrap();

    assert_eq!(env.section().await.cursor_ms, 0);
    assert!(env.cached().is_empty());
    assert!(!env.cache_path().exists());
}

#[tokio::test]
async fn the_keep_alive_swaps_the_refresh_token_and_saves_it() {
    let env = Env::signed_in().await;
    let before = env.section().await.session.unwrap();

    env.omron.keep_alive_once().await.unwrap();

    let after = env.section().await.session.unwrap();

    assert_eq!(env.fake.token_calls("refresh"), 1);
    assert_ne!(after.refresh_token, before.refresh_token);
    assert!(env.section().await.last_refresh_at.is_some());
}

#[tokio::test]
async fn the_keep_alive_does_nothing_without_a_usable_login() {
    let env = Env::start().await;

    env.omron.keep_alive_once().await.unwrap();

    assert!(env.fake.calls().is_empty());

    env.omron.sign_in(COUNTRY, EMAIL, PASSWORD).await.unwrap();
    env.fake.refuse_refresh();

    env.omron.keep_alive_once().await.unwrap_err();

    assert!(env.section().await.login_invalid);

    let calls = env.fake.calls().len();

    env.omron.keep_alive_once().await.unwrap();

    assert_eq!(
        env.fake.calls().len(),
        calls,
        "an invalid login is left alone"
    );
}

#[tokio::test]
async fn the_keep_alive_loop_refreshes_at_start_up_and_then_on_its_interval_until_shutdown() {
    let env = Env::signed_in().await;
    let omron = Omron::new(
        omron_client(&env.fake.base_url),
        env.store.clone(),
        Duration::from_millis(50),
    );
    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let shutdown = shutdown.clone();

        async move { omron.run_keep_alive(shutdown).await }
    });

    wait_until(Duration::from_secs(5), "three refreshes", || {
        env.fake.token_calls("refresh") >= 3
    })
    .await;
    shutdown.cancel();
    task.await.unwrap();

    let refreshes = env.fake.token_calls("refresh");

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        env.fake.token_calls("refresh"),
        refreshes,
        "the loop stopped"
    );
}
