//! A loopback HTTP stub for the tests that fetch natively (`native_fetch.rs`,
//! `native_smoke.rs`), so no test reaches a real host.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use ikigai_core::Capability;

/// An HTTP/1.1 stub on an ephemeral loopback port. `/ok` and `/elsewhere` answer 200,
/// `/hop` answers `302 Location: /elsewhere`, anything else 404.
pub struct Stub {
    pub port: u16,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Stub {
    pub fn start() -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stub");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                // Drain the head; the requests here carry no body.
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if header == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                log.lock().expect("log").push(path.clone());
                let (status, extra, body) = match path.as_str() {
                    "/ok" => ("200 OK", "", "stub ok"),
                    "/elsewhere" => ("200 OK", "", "followed"),
                    "/hop" => ("302 Found", "Location: /elsewhere\r\n", ""),
                    _ => ("404 Not Found", "", "no such path"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{extra}Content-Type: text/plain\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Stub { port, hits }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// A grant for exactly these paths on this stub's port.
    pub fn grant(&self, paths: &[&str]) -> Capability {
        Capability::scoped(
            paths
                .iter()
                .map(|p| format!("urn:cap:net:127.0.0.1:{}{p}", self.port))
                .collect::<Vec<_>>(),
        )
    }

    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("log").clone()
    }
}
