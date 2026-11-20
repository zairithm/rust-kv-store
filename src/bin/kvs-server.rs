//! `kvs-server`: serves a store directory over TCP.
//!
//! ```text
//! kvs-server [--addr 127.0.0.1:4000] [--dir .]
//! ```

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use rust_kv_store::{net, KvStore};

#[derive(Parser)]
#[command(name = "kvs-server", version, about = "Serve a kvs store over TCP")]
struct Args {
    /// Address to listen on. Use port 0 to pick any free port.
    #[arg(long, default_value = "127.0.0.1:4000")]
    addr: String,
    /// Directory holding the store's log.
    #[arg(long, default_value = ".")]
    dir: PathBuf,
}

fn run(args: Args) -> rust_kv_store::Result<()> {
    let store = KvStore::open(&args.dir)?;
    let listener = TcpListener::bind(&args.addr)?;
    // Print the real address (useful with port 0); tests read this line.
    println!("listening on {}", listener.local_addr()?);
    net::serve(listener, store)
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
