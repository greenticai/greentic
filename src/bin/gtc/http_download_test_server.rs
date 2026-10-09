//! A scripted loopback HTTP/1.1 server for download tests.
//!
//! Each accepted connection is answered by the next [`Reply`] in the script,
//! on its own thread, so a connection that is deliberately stalling never
//! delays the accept of the client's retry.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) enum Reply {
    /// The whole body, split into `chunks` writes with `gap` between them.
    Trickle {
        body: Vec<u8>,
        chunks: usize,
        gap: Duration,
    },
    /// Announce the whole body, send `sent` bytes, go silent for `stall`,
    /// then close without sending the rest.
    Stall {
        body: Vec<u8>,
        sent: usize,
        stall: Duration,
    },
    /// Announce the whole body, send `sent` bytes, close immediately.
    Truncate { body: Vec<u8>, sent: usize },
    /// A bodyless response with this status code.
    Status(u16),
}

pub(crate) struct TestServer {
    pub(crate) url: String,
    accepted: Arc<AtomicUsize>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    /// Connections accepted so far.
    pub(crate) fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Wait for every scripted connection to finish.
    pub(crate) fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("test server thread");
        }
    }
}

/// Serve `replies` in order, one per connection, at `/<file_name>`. Once the
/// script is exhausted the listener closes, so an unexpected extra attempt is
/// refused instead of hanging.
pub(crate) fn serve(file_name: &str, replies: Vec<Reply>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepted);
    let handle = thread::spawn(move || {
        let mut workers = Vec::new();
        for reply in replies {
            let (stream, _) = listener.accept().expect("accept");
            counter.fetch_add(1, Ordering::SeqCst);
            workers.push(thread::spawn(move || answer(stream, reply)));
        }
        drop(listener);
        for worker in workers {
            worker.join().expect("connection thread");
        }
    });
    TestServer {
        url: format!("http://{addr}/{file_name}"),
        accepted,
        handle: Some(handle),
    }
}

fn answer(mut stream: TcpStream, reply: Reply) {
    read_request_head(&mut stream);
    // Write errors are expected once the client has timed out and hung up.
    let _ = match reply {
        Reply::Trickle { body, chunks, gap } => {
            let _ = stream.write_all(head(200, body.len()).as_bytes());
            let size = body.len().div_ceil(chunks.max(1)).max(1);
            let mut result = Ok(());
            for piece in body.chunks(size) {
                thread::sleep(gap);
                result = stream.write_all(piece).and_then(|()| stream.flush());
                if result.is_err() {
                    break;
                }
            }
            result
        }
        Reply::Stall { body, sent, stall } => {
            let _ = stream.write_all(head(200, body.len()).as_bytes());
            let _ = stream.write_all(&body[..sent]);
            let _ = stream.flush();
            thread::sleep(stall);
            Ok(())
        }
        Reply::Truncate { body, sent } => {
            let _ = stream.write_all(head(200, body.len()).as_bytes());
            stream.write_all(&body[..sent])
        }
        Reply::Status(code) => stream.write_all(head(code, 0).as_bytes()),
    };
}

fn head(code: u16, content_length: usize) -> String {
    format!(
        "HTTP/1.1 {code} Scripted\r\nContent-Length: {content_length}\r\n\
         Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
    )
}

fn read_request_head(stream: &mut TcpStream) {
    let mut buf = [0_u8; 4096];
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&buf[..read]),
        }
    }
}
