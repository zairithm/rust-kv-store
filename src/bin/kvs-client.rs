//! `kvs-client`: talks to a running `kvs-server`.
//!
//! ```text
//! kvs-client [--addr 127.0.0.1:4000] set <key> <value>
//! kvs-client [--addr 127.0.0.1:4000] get <key>
//! kvs-client [--addr 127.0.0.1:4000] rm <key>
//! ```
//!
//! Exit codes match `kvs`: 0 success, 1 key not found, 2 any other error.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rust_kv_store::net::KvsClient;
use rust_kv_store::Error;

#[derive(Parser)]
#[command(name = "kvs-client", version, about = "Talk to a kvs-server")]
struct Cli {
    /// Server address.
    #[arg(long, default_value = "127.0.0.1:4000", global = true)]
    addr: String,

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
}

fn run(cli: Cli) -> Result<ExitCode, Error> {
    let mut client = KvsClient::connect(&cli.addr)?;
    match cli.command {
        Command::Set { key, value } => client.set(&key, &value)?,
        Command::Get { key } => match client.get(&key)? {
            Some(value) => println!("{value}"),
            None => {
                println!("Key not found");
                return Ok(ExitCode::FAILURE);
            }
        },
        Command::Rm { key } => client.remove(&key)?,
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
