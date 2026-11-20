//! Server and client over real TCP sockets.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;

use rust_kv_store::net::{self, KvsClient};
use rust_kv_store::{Error, KvStore};
use tempfile::TempDir;

/// Starts an in-process server on a free port.
fn start(dir: &TempDir) -> SocketAddr {
    let store = KvStore::open(dir.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || net::serve(listener, store));
    addr
}

#[test]
fn set_get_rm_over_the_network() {
    let dir = TempDir::new().unwrap();
    let mut client = KvsClient::connect(start(&dir)).unwrap();
    client.set("lang", "rust").unwrap();
    assert_eq!(client.get("lang").unwrap().as_deref(), Some("rust"));
    client.remove("lang").unwrap();
    assert_eq!(client.get("lang").unwrap(), None);
    assert!(matches!(client.remove("lang"), Err(Error::KeyNotFound)));
}

#[test]
fn many_clients_share_one_store() {
    let dir = TempDir::new().unwrap();
    let addr = start(&dir);
    let writers: Vec<_> = (0..8)
        .map(|t| {
            thread::spawn(move || {
                let mut c = KvsClient::connect(addr).unwrap();
                for i in 0..50 {
                    c.set(&format!("t{t}-k{i}"), &i.to_string()).unwrap();
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    let mut reader = KvsClient::connect(addr).unwrap();
    for t in 0..8 {
        assert_eq!(
            reader.get(&format!("t{t}-k49")).unwrap().as_deref(),
            Some("49")
        );
    }
}

#[test]
fn data_written_over_the_network_is_on_disk() {
    let dir = TempDir::new().unwrap();
    let mut client = KvsClient::connect(start(&dir)).unwrap();
    client.set("persisted", "yes").unwrap();
    // Open the same directory directly: the record is already in the log.
    let mut store = KvStore::open(dir.path()).unwrap();
    assert_eq!(store.get("persisted").unwrap().as_deref(), Some("yes"));
}

#[test]
fn bad_request_gets_an_error_line_and_keeps_the_connection() {
    let dir = TempDir::new().unwrap();
    let stream = TcpStream::connect(start(&dir)).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    let mut line = String::new();

    writer.write_all(b"not json\n").unwrap();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with(r#"{"err":"bad request"#), "{line}");

    line.clear();
    writer
        .write_all(b"{\"cmd\":\"get\",\"key\":\"x\"}\n")
        .unwrap();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "{\"ok\":null}\n");
}

/// Kills the server process when the test ends, even if it fails.
struct ServerProcess(Child);
impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_binaries_work_together() {
    let dir = TempDir::new().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kvs-server"))
        .args(["--addr", "127.0.0.1:0", "--dir"])
        .arg(dir.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let _server = ServerProcess(child);
    let addr = first
        .trim()
        .strip_prefix("listening on ")
        .unwrap()
        .to_string();

    let client = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_kvs-client"))
            .args(["--addr", &addr])
            .args(args)
            .output()
            .unwrap()
    };
    assert!(client(&["set", "name", "zaid"]).status.success());
    let got = client(&["get", "name"]);
    assert!(got.status.success());
    assert_eq!(String::from_utf8_lossy(&got.stdout), "zaid\n");
    let missing = client(&["rm", "nope"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&missing.stdout), "Key not found\n");
}
