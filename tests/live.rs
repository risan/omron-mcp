//! A manual smoke test against a real OMRON connect account. It only reads, and never runs
//! by default:
//!
//!     OMRON_EMAIL=... OMRON_PASSWORD=... OMRON_COUNTRY=ID \
//!         cargo test --features live --test live -- --nocapture
//!
//! It signs in as a throwaway session in a temporary data dir, so it never touches the
//! service's own login, and prints counts only.
#![cfg(feature = "live")]

mod support;

use mcp_starter::store::{Store, StoreOptions};
use omron_mcp::sync::{KEEP_ALIVE_EVERY, Omron};

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set for the live test"))
}

#[tokio::test]
async fn sign_in_sync_and_read_the_cache() {
    let (email, password, country) = (
        required("OMRON_EMAIL"),
        required("OMRON_PASSWORD"),
        required("OMRON_COUNTRY"),
    );

    support::install_crypto_provider();

    let http = mcp_starter::http::client_builder(None)
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default()).unwrap();
    let omron = Omron::new(omron_connect::Client::new(http), store, KEEP_ALIVE_EVERY);

    omron.sign_in(&country, &email, &password).await.unwrap();

    let added = omron.sync().await.unwrap();
    let readings = omron.load_readings().await.unwrap();

    println!("synced {added} readings; {} cached", readings.len());

    assert_eq!(added, readings.len());

    for reading in &readings {
        assert!(
            reading.systolic > reading.diastolic,
            "systolic above diastolic"
        );
        assert!(reading.measured_at_ms > 0);
    }

    assert_eq!(
        omron.sync().await.unwrap(),
        0,
        "a second sync finds nothing new"
    );
}
