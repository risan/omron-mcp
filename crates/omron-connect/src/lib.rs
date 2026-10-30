//! A client for the OMRON connect cloud, built to read blood pressure
//! readings.
//!
//! ```no_run
//! # async fn demo() -> Result<(), omron_connect::Error> {
//! let client = omron_connect::Client::new(reqwest::Client::new());
//! let mut session = client.login("ID", "me@example.com", "password").await?;
//!
//! if client.has_new(&mut session, 0).await? {
//!     let batch = client.fetch_since(&mut session, 0).await?;
//!     println!("{} readings, next cursor {}", batch.readings.len(), batch.cursor);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! OMRON has two cloud systems, chosen by country (see [`Region`]):
//!
//! - v1 is verified against a live account.
//! - **v2 is untested.** It follows the published notes of the omramin
//!   project and has never run against OMRON's servers.
//!
//! The crate installs no TLS provider: build the `reqwest::Client` with one
//! (reqwest's `rustls-no-provider` feature is used here).
//!
//! Every call that needs a login takes `&mut Session`, refreshes the token
//! when it is close to expiry, and may change the session. Save it after each
//! call. Not affiliated with OMRON Healthcare.

mod error;
mod http;
mod reading;
mod region;
mod session;
mod wire;
pub use error::{Error, Result};
pub use reading::{Batch, Reading, ReadingZone};
pub use region::{Region, V1Region, V2Region};
pub use session::Session;
