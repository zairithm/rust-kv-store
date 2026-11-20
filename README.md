# rust-kv-store

[![CI](https://github.com/zairithm/rust-kv-store/actions/workflows/ci.yml/badge.svg)](https://github.com/zairithm/rust-kv-store/actions/workflows/ci.yml)

A persistent key-value store written from scratch in Rust, built as an append-only log in the style of Bitcask.

## Try it

```console
$ cargo install --path .
$ kvs set name zaid
$ kvs set lang c++
$ kvs set lang rust
$ kvs get lang
rust
$ kvs rm name
$ kvs get name
Key not found
$ kvs stats
keys: 1
stale bytes: 106
$ kvs compact
reclaimed 106 bytes
```

`--dir <path>` picks the store directory (default: the current folder). Exit codes: `0` success, `1` key not found, `2` any other error (for example a corrupt log).

Over the network:

```console
$ kvs-server --addr 127.0.0.1:4000 --dir data &
listening on 127.0.0.1:4000
$ kvs-client set lang rust
$ kvs-client get lang
rust
$ kvs-client rm nope
Key not found
```

`kvs-client` uses the same exit codes as `kvs`. The protocol is one JSON object per line:

```text
→ {"cmd":"set","key":"lang","value":"rust"}    ← {"ok":null}
→ {"cmd":"get","key":"lang"}                   ← {"ok":"rust"}
→ {"cmd":"rm","key":"nope"}                    ← {"err":"Key not found"}
```

The server runs each connection on its own thread; they share one store behind a `Mutex`, so commands never interleave inside the log.

As a library:

```rust
let mut store = rust_kv_store::KvStore::open("data")?;
store.set("lang".into(), "rust".into())?;
assert_eq!(store.get("lang")?.as_deref(), Some("rust"));
store.remove("lang")?;
```

## How it works

```
kvs.log (append-only, one JSON record per line)         index (in memory)
offset   0  {"op":"set","key":"name","value":"zaid"}      lang -> (offset 81, 41 bytes)
offset  41  {"op":"set","key":"lang","value":"c++"}
offset  81  {"op":"set","key":"lang","value":"rust"}
offset 122  {"op":"rm","key":"name"}
```

- **Writes** append one record and flush. Nothing is ever changed in place, so a write can't damage older data.
- **Reads** look up the key's offset and length in the index, then do one seek and one read.
- **Opening** replays the log from the start to rebuild the index.
- **Crash safety:** if the last record was only half written, it is cut off on the next open. A broken record anywhere else stops the open with a `Corrupt` error rather than silently losing data.
- **Stale bytes:** overwritten values, removed values and `rm` records are counted.
- **Compaction** copies only each key's latest record into `kvs.log.compact`, syncs it, then renames it over `kvs.log`. The rename is atomic, so a crash leaves either the old log or the new one; a leftover half-built copy is deleted on the next open. It runs by itself once stale bytes reach 1 MiB and outweigh the live data, or on demand with `kvs compact`.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Unit tests cover the store (round trips, reopening, tricky values, torn writes, corruption, compaction). `tests/cli.rs` runs the real `kvs` binary; `tests/server.rs` covers the protocol, 8 concurrent clients, bad requests, and the `kvs-server` + `kvs-client` binaries together.

## Roadmap

- [x] In-memory store with `set`, `get` and `remove`
- [x] Persist every write to a log file and rebuild the index on start
- [x] Command-line interface
- [x] Log compaction (automatic and `kvs compact`)
- [x] TCP server and client (`kvs-server`, `kvs-client`)
- [ ] Benchmarks and a write-up of the design

## License

MIT
