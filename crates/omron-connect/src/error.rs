use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// The email or password was refused.
    BadCredentials,
    /// The refresh token was refused or the access token still fails after a
    /// refresh: the account must sign in again.
    TokenExpired,
    /// The country code is unknown or OMRON serves it from no cloud.
    UnsupportedCountry(String),
    RateLimited,
    /// OMRON answered with an unexpected HTTP status.
    Upstream {
        status: u16,
    },
    /// The reply grew past the download cap.
    ReplyTooLarge,
    /// The request never completed (DNS, connect, TLS, timeout, broken body).
    Network(String),
    /// The reply arrived but is not shaped as expected. The message never
    /// carries reply content.
    BadReply(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadCredentials => write!(f, "OMRON refused the email or password"),
            Error::TokenExpired => write!(f, "the OMRON login expired: sign in again"),
            Error::UnsupportedCountry(code) => {
                write!(f, "OMRON connect is not available for country \"{code}\"")
            }
            Error::RateLimited => write!(f, "OMRON is rate limiting requests: try again later"),
            Error::Upstream { status } => write!(f, "OMRON answered with HTTP {status}"),
            Error::ReplyTooLarge => write!(f, "the OMRON reply is too large"),
            Error::Network(message) => write!(f, "could not reach OMRON: {message}"),
            Error::BadReply(message) => write!(f, "unexpected OMRON reply: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Error::Network(error.without_url().to_string())
    }
}
