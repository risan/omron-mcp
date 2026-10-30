//! Reading replies without holding them in memory.

use std::io::{self, BufReader, Read};
use std::thread;

use bytes::{Buf, Bytes};
use reqwest::{RequestBuilder, Response};
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc, oneshot};

use crate::error::{Error, Result};

/// Chunks queued between the download and the parser. Small on purpose: the
/// peak buffer is this many chunks plus the one being parsed.
const QUEUE_DEPTH: usize = 2;

/// Stack for the parser thread. serde_json caps nesting at 128 levels.
const PARSER_STACK_BYTES: usize = 512 * 1024;

/// The caller's HTTP client plus the download cap.
#[derive(Clone)]
pub(crate) struct Http {
    pub client: reqwest::Client,
    pub max_reply_bytes: u64,
}

/// Cap for the small replies: sign-in, refresh and the count check.
pub(crate) const SMALL_REPLY_BYTES: u64 = 1024 * 1024;

/// How a status code is judged. `auth_statuses` map to `auth_error`; other
/// non-success statuses become `Upstream`, except 429.
pub(crate) fn check_status(
    response: &Response,
    auth_statuses: &[u16],
    auth_error: Error,
) -> Result<()> {
    let status = response.status().as_u16();

    if response.status().is_success() {
        return Ok(());
    }

    if auth_statuses.contains(&status) {
        return Err(auth_error);
    }

    if status == 429 {
        return Err(Error::RateLimited);
    }

    Err(Error::Upstream { status })
}

pub(crate) async fn send(request: RequestBuilder) -> Result<Response> {
    Ok(request.send().await?)
}

fn reject_declared_size(response: &Response, cap: u64) -> Result<()> {
    match response.content_length() {
        Some(length) if length > cap => Err(Error::ReplyTooLarge),
        _ => Ok(()),
    }
}

/// Reads a small reply into memory, up to `cap` bytes, and parses it.
pub(crate) async fn read_json<T: DeserializeOwned>(mut response: Response, cap: u64) -> Result<T> {
    reject_declared_size(&response, cap)?;

    let mut body = Vec::new();

    while let Some(chunk) = response.chunk().await? {
        if (body.len() + chunk.len()) as u64 > cap {
            return Err(Error::ReplyTooLarge);
        }

        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body).map_err(bad_reply)
}

/// Parses a reply of any size while it downloads. The body flows through a
/// small bounded queue into a parser thread, so memory stays flat however
/// large the reply is. A thread that ends with the parse (instead of a
/// `spawn_blocking` task) leaves no idle worker behind in the caller's
/// runtime.
pub(crate) async fn read_json_streaming<T>(mut response: Response, cap: u64) -> Result<T>
where
    T: DeserializeOwned + Send + 'static,
{
    reject_declared_size(&response, cap)?;

    let (chunk_tx, chunk_rx) = mpsc::channel::<Bytes>(QUEUE_DEPTH);
    let (result_tx, result_rx) = oneshot::channel();

    thread::Builder::new()
        .name("omron-parse".into())
        .stack_size(PARSER_STACK_BYTES)
        .spawn(move || {
            let reader = BufReader::with_capacity(
                8 * 1024,
                ChunkReader {
                    chunks: chunk_rx,
                    current: Bytes::new(),
                },
            );
            let _ = result_tx.send(serde_json::from_reader::<_, T>(reader));
        })
        .map_err(|error| Error::Network(format!("could not start the reply parser: {error}")))?;

    let mut received: u64 = 0;
    let mut download_error = None;

    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                received += chunk.len() as u64;

                if received > cap {
                    download_error = Some(Error::ReplyTooLarge);
                    break;
                }

                if chunk_tx.send(chunk).await.is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                download_error = Some(error.into());
                break;
            }
        }
    }

    drop(chunk_tx);

    if let Some(error) = download_error {
        return Err(error);
    }

    match result_rx.await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(bad_reply(error)),
        Err(_) => Err(Error::BadReply("the reply parser stopped".into())),
    }
}

fn bad_reply(error: serde_json::Error) -> Error {
    Error::BadReply(format!(
        "{:?} at line {} column {}",
        error.classify(),
        error.line(),
        error.column()
    ))
}

struct ChunkReader {
    chunks: mpsc::Receiver<Bytes>,
    current: Bytes,
}

impl Read for ChunkReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.current.is_empty() {
            match self.chunks.blocking_recv() {
                Some(chunk) => self.current = chunk,
                None => return Ok(0),
            }
        }

        let count = buffer.len().min(self.current.len());
        buffer[..count].copy_from_slice(&self.current[..count]);
        self.current.advance(count);

        Ok(count)
    }
}
