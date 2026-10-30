use crate::error::{Error, Result};
use crate::http::Http;
use crate::reading::Batch;
use crate::region::Region;
use crate::session::Session;
use crate::{v1, v2};

/// The default cap on one reply. A first sync of years of readings on a
/// pulse-wave cuff can run to hundreds of megabytes, so it is generous.
const DEFAULT_MAX_REPLY_BYTES: u64 = 1024 * 1024 * 1024;

/// Talks to the OMRON cloud for one account at a time.
///
/// The `reqwest::Client` is the caller's, so the caller decides TLS, proxies
/// and timeouts. Keep its timeout generous: a long first sync is one slow
/// download. Do not enable response compression: OMRON's v1 cloud can label
/// plain bodies as gzip.
#[derive(Clone)]
pub struct Client {
    http: Http,
    base_urls: Option<Vec<String>>,
}

impl Client {
    pub fn new(client: reqwest::Client) -> Self {
        Client {
            http: Http {
                client,
                max_reply_bytes: DEFAULT_MAX_REPLY_BYTES,
            },
            base_urls: None,
        }
    }

    /// Replaces the API roots from the region table, mainly for tests. A v1
    /// region uses the first one; a v2 region tries them in order.
    pub fn with_base_urls<I, S>(mut self, urls: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base_urls = Some(
            urls.into_iter()
                .map(|url| url.into().trim_end_matches('/').to_owned())
                .collect(),
        );
        self
    }

    /// Lowers or raises the cap on one reply. Past it, a call fails with
    /// `Error::ReplyTooLarge`.
    pub fn with_max_reply_bytes(mut self, bytes: u64) -> Self {
        self.http.max_reply_bytes = bytes;
        self
    }

    pub async fn login(&self, country: &str, email: &str, password: &str) -> Result<Session> {
        let country = country.trim().to_ascii_uppercase();
        let (base_url, tokens) = match Region::for_country(&country)? {
            Region::V1(region) => {
                let base_url = self
                    .base_url_override()
                    .unwrap_or_else(|| region.cloud_url.to_owned());
                let tokens = v1::login(&self.http, region, &base_url, email, password).await?;

                (base_url, tokens)
            }
            Region::V2(region) => {
                let base_urls = match &self.base_urls {
                    Some(urls) => urls.clone(),
                    None => region
                        .base_urls
                        .iter()
                        .map(|url| (*url).to_owned())
                        .collect(),
                };

                v2::login(&self.http, &base_urls, &country, email, password).await?
            }
        };

        let mut session = Session {
            country,
            email: email.to_owned(),
            base_url,
            access_token: String::new(),
            access_expires_at_ms: 0,
            refresh_token: String::new(),
        };
        session.apply(tokens);

        Ok(session)
    }

    /// Swaps the refresh token for a new access token. The other calls do
    /// this by themselves when the access token is missing or about to expire.
    pub async fn refresh(&self, session: &mut Session) -> Result<()> {
        let tokens = match Region::for_country(&session.country)? {
            Region::V1(region) => v1::refresh(&self.http, region, session).await?,
            Region::V2(_) => v2::refresh(&self.http, session).await?,
        };
        session.apply(tokens);

        Ok(())
    }

    /// A cheap check for readings newer than `cursor`. Only v1 can answer
    /// without fetching; v2 always says yes. When the cloud's answer is
    /// unclear it also says yes.
    pub async fn has_new(&self, session: &mut Session, cursor: i64) -> Result<bool> {
        let Region::V1(region) = Region::for_country(&session.country)? else {
            return Ok(true);
        };

        self.ensure_fresh(session).await?;

        match v1::has_new(&self.http, region, session, cursor).await {
            Err(Error::TokenExpired) => {
                self.refresh(session).await?;
                v1::has_new(&self.http, region, session, cursor).await
            }
            other => other,
        }
    }

    /// Fetches what is new since `cursor` (0 for everything). The reply is
    /// parsed while it downloads, so memory stays flat however large it is.
    /// Keep the old cursor if this fails; use `Batch::cursor` if it works.
    pub async fn fetch_since(&self, session: &mut Session, cursor: i64) -> Result<Batch> {
        self.ensure_fresh(session).await?;

        match self.fetch_once(session, cursor).await {
            Err(Error::TokenExpired) => {
                self.refresh(session).await?;
                self.fetch_once(session, cursor).await
            }
            other => other,
        }
    }

    async fn fetch_once(&self, session: &Session, cursor: i64) -> Result<Batch> {
        match Region::for_country(&session.country)? {
            Region::V1(region) => v1::fetch(&self.http, region, session, cursor).await,
            Region::V2(_) => v2::fetch(&self.http, session, cursor).await,
        }
    }

    async fn ensure_fresh(&self, session: &mut Session) -> Result<()> {
        if session.needs_refresh() {
            self.refresh(session).await?;
        }

        Ok(())
    }

    fn base_url_override(&self) -> Option<String> {
        self.base_urls.as_ref()?.first().cloned()
    }
}
