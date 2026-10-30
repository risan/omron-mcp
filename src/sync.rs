//! The OMRON account behind the tools and the CLI: it loads the saved login, calls OMRON,
//! saves the login again when a call changed it, and merges new readings into the cache.
//! Nothing is held in memory between calls.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use mcp_starter::store::{Change, Store};
use omron_connect::{Client, Error, Reading, Session};
use tokio_util::sync::CancellationToken;

use crate::cache::ReadingCache;
use crate::store::OmronState;

pub const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub enum SyncError {
    NotSignedIn,
    /// OMRON refused the refresh token; only a new `omron-mcp login` helps.
    LoginInvalid,
    Omron(Error),
    Store(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::NotSignedIn => write!(f, "not signed in to OMRON: run `omron-mcp login`"),
            SyncError::LoginInvalid => {
                write!(f, "the OMRON login expired: run `omron-mcp login`")
            }
            SyncError::Omron(error) => write!(f, "{error}"),
            SyncError::Store(message) => write!(f, "{message}"),
        }
    }
}

impl SyncError {
    /// Whether the owner has to sign in again before a sync can work.
    pub fn needs_login(&self) -> bool {
        matches!(self, SyncError::NotSignedIn | SyncError::LoginInvalid)
    }
}

/// The saved login a call starts from. `rev` guards the save: a login made meanwhile wins.
struct Login {
    session: Session,
    cursor_ms: i64,
    rev: i64,
}

#[derive(Clone)]
pub struct Omron {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    store: Store<OmronState>,
    cache: ReadingCache,
    keep_alive_every: Duration,
    /// One call to OMRON at a time, so two refreshes never race for the refresh token.
    busy: tokio::sync::Mutex<()>,
}

impl Omron {
    pub fn new(client: Client, store: Store<OmronState>, keep_alive_every: Duration) -> Omron {
        Omron {
            inner: Arc::new(Inner {
                client,
                cache: ReadingCache::new(&store),
                store,
                keep_alive_every,
                busy: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Signs in and saves the login. Another account starts with an empty cache.
    pub async fn sign_in(
        &self,
        country: &str,
        email: &str,
        password: &str,
    ) -> Result<(), SyncError> {
        let session = self
            .inner
            .client
            .login(country, email, password)
            .await
            .map_err(SyncError::Omron)?;
        let cache = self.inner.cache.clone();

        self.inner
            .store
            .update(move |state| {
                let section = &mut state.section;
                let same_account = section.session.as_ref().is_some_and(|saved| {
                    saved.country == session.country
                        && saved.email.eq_ignore_ascii_case(&session.email)
                });

                if !same_account {
                    section.cursor_ms = 0;

                    if let Err(error) = cache.clear() {
                        return Change::Unchanged(Err(error));
                    }
                }

                section.session = Some(session);
                section.login_rev += 1;
                section.login_invalid = false;
                section.last_refresh_at = Some(Timestamp::now());

                Change::Write(Ok(()))
            })
            .await
            .map_err(|error| SyncError::Store(error.to_string()))?
            .map_err(SyncError::Store)
    }

    /// Fetches what OMRON has newer than the saved cursor and merges it into the cache. It
    /// asks for the readings only when a cheap check says there are new ones. Returns how many
    /// readings the cache did not have.
    pub async fn sync(&self) -> Result<usize, SyncError> {
        let _busy = self.inner.busy.lock().await;
        let login = self.read_login().await?;
        let rev = login.rev;
        let cursor = login.cursor_ms;
        let batch = self
            .with_session(login, async |client, session| {
                if !client.has_new(session, cursor).await? {
                    return Ok(None);
                }

                client.fetch_since(session, cursor).await.map(Some)
            })
            .await?;
        let Some(batch) = batch else {
            return Ok(0);
        };
        let cache = self.inner.cache.clone();

        self.inner
            .store
            .update(move |state| {
                if state.section.login_rev != rev {
                    return Change::Unchanged(Ok(0));
                }

                match cache.merge(batch.readings, &batch.deleted) {
                    Ok(merged) => {
                        state.section.cursor_ms = state.section.cursor_ms.max(batch.cursor);

                        Change::Write(Ok(merged.added))
                    }
                    Err(error) => Change::Unchanged(Err(error)),
                }
            })
            .await
            .map_err(|error| SyncError::Store(error.to_string()))?
            .map_err(SyncError::Store)
    }

    /// Trades the refresh token for a fresh pair, which keeps an unused login alive. Does
    /// nothing without a usable login.
    pub async fn keep_alive_once(&self) -> Result<(), SyncError> {
        let _busy = self.inner.busy.lock().await;
        let login = match self.read_login().await {
            Ok(login) => login,
            Err(error) if error.needs_login() => return Ok(()),
            Err(error) => return Err(error),
        };

        self.with_session(login, async |client, session| client.refresh(session).await)
            .await
    }

    /// Runs `keep_alive_once` at start-up and then every 24 hours until `shutdown`. The loop
    /// stops between refreshes, never inside one.
    pub async fn run_keep_alive(&self, shutdown: CancellationToken) {
        while !shutdown.is_cancelled() {
            if let Err(error) = self.keep_alive_once().await {
                eprintln!("omron keep-alive: {error}");
            }

            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(self.inner.keep_alive_every) => {}
            }
        }
    }

    pub async fn load_readings(&self) -> Result<Vec<Reading>, SyncError> {
        let cache = self.inner.cache.clone();

        tokio::task::spawn_blocking(move || cache.load())
            .await
            .map_err(|error| SyncError::Store(format!("read the readings cache: {error}")))?
            .map_err(SyncError::Store)
    }

    async fn read_login(&self) -> Result<Login, SyncError> {
        let (session, cursor_ms, rev, invalid) = self
            .inner
            .store
            .read(|state| {
                let section = &state.section;

                (
                    section.session.clone(),
                    section.cursor_ms,
                    section.login_rev,
                    section.login_invalid,
                )
            })
            .await
            .map_err(|error| SyncError::Store(error.to_string()))?;

        match session {
            None => Err(SyncError::NotSignedIn),
            Some(_) if invalid => Err(SyncError::LoginInvalid),
            Some(session) => Ok(Login {
                session,
                cursor_ms,
                rev,
            }),
        }
    }

    /// Runs `call` with the login's session and saves the session again when the call changed
    /// it (a token refresh), even if the call failed. A refused refresh marks the login invalid.
    async fn with_session<T>(
        &self,
        login: Login,
        call: impl AsyncFnOnce(&Client, &mut Session) -> Result<T, Error>,
    ) -> Result<T, SyncError> {
        let mut session = login.session.clone();
        let result = call(&self.inner.client, &mut session).await;
        let expired = matches!(result, Err(Error::TokenExpired));

        if expired || session != login.session {
            self.save_session(session, login.rev, expired).await?;
        }

        match result {
            Err(_) if expired => Err(SyncError::LoginInvalid),
            other => other.map_err(SyncError::Omron),
        }
    }

    async fn save_session(
        &self,
        session: Session,
        rev: i64,
        expired: bool,
    ) -> Result<(), SyncError> {
        self.inner
            .store
            .update(move |state| {
                let section = &mut state.section;

                if section.login_rev != rev {
                    return Change::Unchanged(());
                }

                if expired {
                    section.login_invalid = true;
                } else {
                    section.session = Some(session);
                    section.last_refresh_at = Some(Timestamp::now());
                }

                Change::Write(())
            })
            .await
            .map_err(|error| SyncError::Store(error.to_string()))
    }
}
