//! A siteverify on the loopback interface that records every request it receives.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

pub(crate) struct Received {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) content_type: Option<String>,
    /// The fields of the form in the order they arrived, decoded.
    pub(crate) form: Vec<(String, String)>,
}

#[derive(Clone)]
pub(crate) enum Mode {
    /// Answers with the status and the body, delimited by Content-Length.
    Answer(u16, String),
    /// Sends the bytes as they are, then closes the connection.
    Raw(Vec<u8>),
    /// Receives requests and never answers them while it runs.
    Silent,
    /// Sends the status, the headers and the start of a chunked body, then nothing more while it runs.
    Stall(u16, String),
    /// Sends the status, the headers and the start of a chunked body, then a space every 100 ms until the client goes.
    Trickle(u16, String),
}

pub(crate) struct LocalSiteverify {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<Received>>>,
    broken_at: Arc<Mutex<Option<Instant>>>,
    task: JoinHandle<()>,
}

impl LocalSiteverify {
    pub(crate) async fn start(mode: Mode) -> LocalSiteverify {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broken_at = Arc::new(Mutex::new(None));

        let (shared_requests, shared_broken_at) = (Arc::clone(&requests), Arc::clone(&broken_at));
        let task = tokio::spawn(async move {
            // Dropped with the task, which aborts every connection still open.
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                connections.spawn(serve(
                    stream,
                    mode.clone(),
                    Arc::clone(&shared_requests),
                    Arc::clone(&shared_broken_at),
                ));
            }
        });

        LocalSiteverify {
            address,
            requests,
            broken_at,
            task,
        }
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn requests(&self) -> std::sync::MutexGuard<'_, Vec<Received>> {
        self.requests.lock().unwrap()
    }

    /// When a trickling server found the connection closed.
    pub(crate) fn broken_at(&self) -> Option<Instant> {
        *self.broken_at.lock().unwrap()
    }
}

impl Drop for LocalSiteverify {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// An address of the loopback interface where nothing listens.
pub(crate) async fn refusing_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}

async fn serve(
    mut stream: TcpStream,
    mode: Mode,
    requests: Arc<Mutex<Vec<Received>>>,
    broken_at: Arc<Mutex<Option<Instant>>>,
) {
    let Some(received) = receive(&mut stream).await else {
        return;
    };
    requests.lock().unwrap().push(received);

    match mode {
        Mode::Answer(status, body) => {
            let answer = format!(
                "HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
        Mode::Raw(bytes) => {
            let _ = stream.write_all(&bytes).await;
            let _ = stream.shutdown().await;
        }
        Mode::Silent => std::future::pending().await,
        Mode::Stall(status, start) => {
            let _ = stream
                .write_all(chunked_start(status, &start).as_bytes())
                .await;
            std::future::pending().await
        }
        Mode::Trickle(status, start) => {
            let _ = stream
                .write_all(chunked_start(status, &start).as_bytes())
                .await;
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if stream.write_all(b"1\r\n \r\n").await.is_err() {
                    *broken_at.lock().unwrap() = Some(Instant::now());
                    return;
                }
            }
        }
    }
}

fn chunked_start(status: u16, start: &str) -> String {
    format!(
        "HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{start}\r\n",
        start.len()
    )
}

async fn receive(stream: &mut TcpStream) -> Option<Received> {
    let mut data = Vec::new();
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let mut chunk = [0; 4096];
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8(data[..head_end].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap().split(' ');
    let method = request_line.next().unwrap().to_string();
    let target = request_line.next().unwrap().to_string();

    let mut content_type = None;
    let mut length = 0;
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.trim().to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap();
        }
    }

    let mut body = data[head_end + 4..].to_vec();
    while body.len() < length {
        let mut chunk = [0; 4096];
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Some(Received {
        method,
        target,
        content_type,
        form: decode_form(&String::from_utf8(body).unwrap()),
    })
}

fn decode_form(body: &str) -> Vec<(String, String)> {
    if body.is_empty() {
        return Vec::new();
    }

    body.split('&')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap();
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let mut bytes = Vec::new();
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        match byte {
            b'+' => {
                bytes.push(b' ');
                rest = tail;
            }
            b'%' => {
                let hex = std::str::from_utf8(&tail[..2]).unwrap();
                bytes.push(u8::from_str_radix(hex, 16).unwrap());
                rest = &tail[2..];
            }
            _ => {
                bytes.push(byte);
                rest = tail;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}
