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
```

`--dir <path>` picks the store directory (default: the current folder). Exit codes: `0` success, `1` key not found, `2` any other error (for example a corrupt log).

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
- **Stale bytes:** overwritten values, removed values and `rm` records are counted, so compaction knows when the log is worth rewriting.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Unit tests cover the store (round trips, reopening, tricky values, torn writes, corruption). Integration tests in `tests/cli.rs` run the real `kvs` binary.

## Roadmap

- [x] In-memory store with `set`, `get` and `remove`
- [x] Persist every write to a log file and rebuild the index on start
- [x] Command-line interface
- [ ] Log compaction
- [ ] TCP server and client
- [ ] Benchmarks and a write-up of the design

## License

MIT
