//! `kvs`: command-line access to a store directory.
//!
//! ```text
//! kvs set <key> <value>
//! kvs get <key>        prints the value, or "Key not found" (exit code 1)
//! kvs rm <key>         "Key not found" (exit code 1) if it isn't there
//! kvs stats            number of keys and reclaimable bytes
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rust_kv_store::{Error, KvStore};

#[derive(Parser)]
#[command(name = "kvs", version, about = "A persistent key-value store")]
struct Cli {
    /// Directory holding the store's log.
    #[arg(long, default_value = ".", global = true)]
    dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set a key to a value.
    Set { key: String, value: String },
    /// Print the value of a key.
    Get { key: String },
    /// Remove a key.
    Rm { key: String },
    /// Show how many keys there are and how much of the log is stale.
    Stats,
}

fn run(cli: Cli) -> Result<ExitCode, Error> {
    let mut store = KvStore::open(&cli.dir)?;
    match cli.command {
        Command::Set { key, value } => store.set(key, value)?,
        Command::Get { key } => match store.get(&key)? {
            Some(value) => println!("{value}"),
            None => {
                println!("Key not found");
                return Ok(ExitCode::FAILURE);
            }
        },
        Command::Rm { key } => store.remove(&key)?,
        Command::Stats => {
            println!("keys: {}", store.len());
            println!("stale bytes: {}", store.stale_bytes());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(Error::KeyNotFound) => {
            println!("Key not found");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
