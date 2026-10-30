//! Peak heap must not grow with the size of the sync reply.
//!
//! A raw TCP server streams about 30 MB of chunked JSON: 100 readings, each
//! followed by a 300 KB pulse-wave record. A counting allocator watches the
//! whole process, server and parser thread included. This file holds one test
//! so nothing else allocates while it runs.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use omron_connect::{Client, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const READINGS: i64 = 100;
const PULSE_WAVE_BYTES: usize = 300 * 1024;
const PEAK_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;
const T0: i64 = 1_700_000_000_000;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

fn grow(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;

    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };

        if !pointer.is_null() {
            grow(layout.size());
        }

        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };

        if !pointer.is_null() {
            grow(layout.size());
        }

        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, new_size) };

        if !moved.is_null() {
            if new_size >= layout.size() {
                grow(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }

        moved
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Writes an HTTP/1.1 chunked body in pieces of about `CHUNK_BYTES`.
struct ChunkedBody {
    stream: TcpStream,
    pending: Vec<u8>,
    sent: usize,
}

impl ChunkedBody {
    async fn push(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.pending.extend_from_slice(bytes);

        if self.pending.len() >= CHUNK_BYTES {
            self.flush().await?;
        }

        Ok(())
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let header = format!("{:x}\r\n", self.pending.len());
        self.stream.write_all(header.as_bytes()).await?;
        self.stream.write_all(&self.pending).await?;
        self.stream.write_all(b"\r\n").await?;
        self.sent += self.pending.len();
        self.pending.clear();

        Ok(())
    }

    async fn finish(mut self) -> std::io::Result<usize> {
        self.flush().await?;
        self.stream.write_all(b"0\r\n\r\n").await?;

        Ok(self.sent)
    }
}

fn item(category: &str, measured_at_ms: i64, body_index_list_start: &str) -> String {
    format!(
        r#"{{"objectId":"fixture-object","transferStartDate":{T0},"deviceCategoryList":[{{"deviceCategory":"{category}","deviceModelList":[{{"deviceModel":"TEST-MODEL-A","deviceSerialIDList":[{{"deviceSerialID":"TEST-SERIAL-0001","userNumberInDevice":1,"measureList":[{{"timeZone":"Asia/Jakarta","deleteFlag":0,"measureDateFrom":{measured_at_ms},"measureDateTo":{measured_at_ms},"measurementMode":0,"bodyIndexList":{{{body_index_list_start}"#
    )
}

const ITEM_END: &str = "}}]}]}]}]}";

async fn serve_one_reply(listener: TcpListener) -> std::io::Result<usize> {
    let (mut stream, _) = listener.accept().await?;
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];

    let header_end = loop {
        let count = stream.read(&mut buffer).await?;

        if count == 0 {
            return Ok(0);
        }

        request.extend_from_slice(&buffer[..count]);

        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };

    let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
    let content_length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);

    while request.len() < header_end + content_length {
        let count = stream.read(&mut buffer).await?;

        if count == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..count]);
    }

    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        .await?;

    let mut body = ChunkedBody {
        stream,
        pending: Vec::new(),
        sent: 0,
    };
    let samples = "\"1234\",".repeat(1000);
    let sample_groups = PULSE_WAVE_BYTES / samples.len();

    body.push(
        format!(
            r#"{{"returnedValue":{{"lastSyncDate":{},"syncList":["#,
            T0 + READINGS
        )
        .as_bytes(),
    )
    .await?;

    for reading in 0..READINGS {
        let measured_at_ms = T0 + reading;
        let blood_pressure = format!(
            r#""1":["{}","mmHg","0","1"],"2":["{}","mmHg","0","2"],"3":["{}","bpm","0","3"],"8":["1","","0","8"]"#,
            120 + reading % 20,
            70 + reading % 15,
            60 + reading % 25
        );

        if reading > 0 {
            body.push(b",").await?;
        }

        body.push(item("0", measured_at_ms, &blood_pressure).as_bytes())
            .await?;
        body.push(ITEM_END.as_bytes()).await?;
        body.push(b",").await?;
        body.push(item("61440", measured_at_ms, r#""ppwData":["#).as_bytes())
            .await?;

        for _ in 0..sample_groups {
            body.push(samples.as_bytes()).await?;
        }

        body.push(b"\"0\"]").await?;
        body.push(ITEM_END.as_bytes()).await?;
    }

    body.push(b"]}}").await?;
    body.finish().await
}

#[tokio::test]
async fn peak_heap_stays_flat_while_30_megabytes_stream_through() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(serve_one_reply(listener));

    let client = Client::new(reqwest::Client::new()).with_base_urls([base_url.clone()]);
    let mut session = Session {
        country: "ID".into(),
        email: "reader@example.test".into(),
        base_url,
        access_token: "fixture-access".into(),
        access_expires_at_ms: i64::MAX,
        refresh_token: "fixture-refresh".into(),
    };

    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
    let baseline = LIVE.load(Ordering::Relaxed);

    let batch = client.fetch_since(&mut session, 0).await.unwrap();

    let peak = PEAK.load(Ordering::Relaxed);
    let served = server.await.unwrap().unwrap();

    eprintln!("served={served} bytes baseline_heap={baseline} peak_heap={peak}");

    assert_eq!(batch.readings.len(), READINGS as usize);
    assert_eq!(batch.cursor, T0 + READINGS);
    assert!(
        served > 30_000_000,
        "the fake reply was only {served} bytes"
    );
    assert!(
        peak < PEAK_LIMIT_BYTES,
        "peak heap {peak} bytes is over the {PEAK_LIMIT_BYTES} byte limit"
    );
}
