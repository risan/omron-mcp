use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Refresh the access token when it has less than this left.
const REFRESH_MARGIN_MS: i64 = 60_000;

/// A signed-in OMRON account. The password is never kept.
///
/// Every call that needs a login takes `&mut Session` and may refresh it, so
/// save the session again after each call.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// Upper-case ISO country code the account signed in with.
    pub country: String,
    pub email: String,
    /// The API root that accepted the login.
    pub base_url: String,
    #[serde(default)]
    pub access_token: String,
    /// Epoch milliseconds when the access token stops working.
    #[serde(default)]
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
}

impl Session {
    pub(crate) fn needs_refresh(&self) -> bool {
        self.access_token.is_empty() || now_ms() + REFRESH_MARGIN_MS >= self.access_expires_at_ms
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("country", &self.country)
            .field("base_url", &self.base_url)
            .field("access_expires_at_ms", &self.access_expires_at_ms)
            .finish_non_exhaustive()
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// What a sign-in or refresh call returns.
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_secs: i64,
}

impl Session {
    pub(crate) fn apply(&mut self, tokens: Tokens) {
        self.access_token = tokens.access_token;
        self.access_expires_at_ms = now_ms() + tokens.expires_in_secs * 1000;
        self.refresh_token = tokens.refresh_token;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(access_token: &str, expires_in_ms: i64) -> Session {
        Session {
            country: "ID".into(),
            email: "reader@example.test".into(),
            base_url: "https://example.test/api".into(),
            access_token: access_token.into(),
            access_expires_at_ms: now_ms() + expires_in_ms,
            refresh_token: "fixture-refresh".into(),
        }
    }

    #[test]
    fn a_token_is_refreshed_only_inside_the_last_minute() {
        assert!(!session("fixture-access", 120_000).needs_refresh());
        assert!(session("fixture-access", 30_000).needs_refresh());
        assert!(session("fixture-access", -1).needs_refresh());
    }

    #[test]
    fn a_missing_token_needs_a_refresh() {
        assert!(session("", 3_600_000).needs_refresh());
    }

    #[test]
    fn a_session_survives_a_json_round_trip() {
        let original = session("fixture-access", 3_600_000);

        let json = serde_json::to_string(&original).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.access_token, original.access_token);
        assert_eq!(restored.refresh_token, original.refresh_token);
        assert_eq!(restored.access_expires_at_ms, original.access_expires_at_ms);
        assert_eq!(restored.base_url, original.base_url);
    }

    #[test]
    fn a_session_saved_without_an_access_token_loads() {
        let json = r#"{"country":"ID","email":"a@example.test","base_url":"https://example.test","refresh_token":"fixture-refresh"}"#;

        let restored: Session = serde_json::from_str(json).unwrap();

        assert!(restored.needs_refresh());
    }
}
