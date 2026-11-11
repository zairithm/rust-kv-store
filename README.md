# rust-kv-store

[![CI](https://github.com/zairithm/rust-kv-store/actions/workflows/ci.yml/badge.svg)](https://github.com/zairithm/rust-kv-store/actions/workflows/ci.yml)

A key-value database written from scratch: first in memory, then a log-structured store on disk, then a server that many clients can use.

## What this project covers

- Designing a library API and error types
- File I/O, serialisation and a write-ahead log
- Log compaction
- A TCP client/server protocol
- Benchmarks with `criterion`

## Getting started

```bash
cargo test
```

## Roadmap

Each open item is tracked as a [GitHub issue](https://github.com/zairithm/rust-kv-store/issues).

- [x] In-memory store with `set`, `get` and `remove`
- [ ] Command-line interface
- [ ] Persist every write to a log file and rebuild the index on start
- [ ] Log compaction
- [ ] TCP server and client
- [ ] Benchmarks and a write-up of the design

## License

MIT
