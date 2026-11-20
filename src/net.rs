//! Network access: a small line-based JSON protocol over TCP.
//!
//! Each request and each response is one JSON object on its own line:
//!
//! ```text
//! → {"cmd":"set","key":"lang","value":"rust"}    ← {"ok":null}
//! → {"cmd":"get","key":"lang"}                   ← {"ok":"rust"}
//! → {"cmd":"get","key":"nope"}                   ← {"ok":null}
//! → {"cmd":"rm","key":"nope"}                    ← {"err":"Key not found"}
//! ```
//!
//! The server handles each connection on its own thread. They share one
//! [`KvStore`] behind a mutex, so commands run one at a time and never interleave
//! inside the log. A client can send many requests over one connection.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::{Error, KvStore, Result};

/// A command sent by a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Get { key: String },
    Set { key: String, value: String },
    Rm { key: String },
}

/// The server's answer: a value (or `null`), or an error message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Response {
    Ok(Option<String>),
    Err(String),
}

/// Message sent back when a key doesn't exist; the client turns it into
/// [`Error::KeyNotFound`].
const KEY_NOT_FOUND: &str = "Key not found";

/// Serves `store` to every client that connects to `listener`, forever.
pub fn serve(listener: TcpListener, store: KvStore) -> Result<()> {
    let store = Arc::new(Mutex::new(store));
    for stream in listener.incoming() {
        let stream = stream?;
        let store = Arc::clone(&store);
        thread::spawn(move || {
            let peer = stream.peer_addr().ok();
            if let Err(e) = handle(stream, &store) {
                eprintln!("{peer:?}: {e}");
            }
        });
    }
    Ok(())
}

fn handle(stream: TcpStream, store: &Mutex<KvStore>) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => execute(store, request),
            Err(e) => Response::Err(format!("bad request: {e}")),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

fn execute(store: &Mutex<KvStore>, request: Request) -> Response {
    // A panic in another connection poisons the lock; the store itself is still
    // consistent because every write is a single append, so keep serving.
    let mut store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = match request {
        Request::Get { key } => store.get(&key),
        Request::Set { key, value } => store.set(key, value).map(|()| None),
        Request::Rm { key } => store.remove(&key).map(|()| None),
    };
    match result {
        Ok(value) => Response::Ok(value),
        Err(Error::KeyNotFound) => Response::Err(KEY_NOT_FOUND.into()),
        Err(e) => Response::Err(e.to_string()),
    }
}

/// A connection to a `kvs-server`.
pub struct KvsClient {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl KvsClient {
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: BufWriter::new(stream),
        })
    }

    pub fn get(&mut self, key: &str) -> Result<Option<String>> {
        self.send(&Request::Get { key: key.into() })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        self.send(&Request::Set {
            key: key.into(),
            value: value.into(),
        })
        .map(|_| ())
    }

    pub fn remove(&mut self, key: &str) -> Result<()> {
        self.send(&Request::Rm { key: key.into() }).map(|_| ())
    }

    fn send(&mut self, request: &Request) -> Result<Option<String>> {
        serde_json::to_writer(&mut self.writer, request)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;

        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(Error::Server("connection closed".into()));
        }
        match serde_json::from_str(&line)? {
            Response::Ok(value) => Ok(value),
            Response::Err(msg) if msg == KEY_NOT_FOUND => Err(Error::KeyNotFound),
            Response::Err(msg) => Err(Error::Server(msg)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_stable() {
        let req = Request::Set {
            key: "k".into(),
            value: "v".into(),
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"cmd":"set","key":"k","value":"v"}"#
        );
        assert_eq!(
            serde_json::to_string(&Response::Ok(Some("v".into()))).unwrap(),
            r#"{"ok":"v"}"#
        );
        assert_eq!(
            serde_json::to_string(&Response::Ok(None)).unwrap(),
            r#"{"ok":null}"#
        );
        assert_eq!(
            serde_json::to_string(&Response::Err("x".into())).unwrap(),
            r#"{"err":"x"}"#
        );
    }
}
