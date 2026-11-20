//! A persistent key-value store, built as an append-only log (the Bitcask idea).
//!
//! Every `set` and `rm` is appended to `kvs.log` as one JSON line. Nothing in the
//! file is ever changed in place. In memory we only keep an index from each key to
//! the position of its latest `set` record, so a `get` is one seek and one read.
//!
//! On `open`, the log is replayed from the start to rebuild the index. If the very
//! last record was only half written (a crash in the middle of a write), it is cut
//! off; a broken record anywhere else is reported as corruption.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod error;

pub use error::{Error, Result};

/// Name of the log file inside the store's directory.
pub const LOG_FILE: &str = "kvs.log";

/// One line of the log.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Record {
    Set { key: String, value: String },
    Rm { key: String },
}

/// Where a record sits in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    offset: u64,
    len: u64,
}

/// A string key-value store kept in a directory on disk.
#[derive(Debug)]
pub struct KvStore {
    path: PathBuf,
    reader: BufReader<File>,
    writer: BufWriter<File>,
    /// Length of the log, which is where the next record goes.
    write_pos: u64,
    /// Key -> its latest `set` record.
    index: HashMap<String, Pos>,
    /// Bytes in the log that no longer matter (overwritten or removed values and
    /// the `rm` records themselves). Compaction will reclaim them.
    stale_bytes: u64,
}

impl KvStore {
    /// Opens the store in `dir`, creating the directory and log if needed.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let path = dir.join(LOG_FILE);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;

        let replay = replay(BufReader::new(&file))?;
        if replay.good_len < file.metadata()?.len() {
            // Drop a half-written last record so new writes start on a clean line.
            file.set_len(replay.good_len)?;
        }

        Ok(Self {
            reader: BufReader::new(File::open(&path)?),
            writer: BufWriter::new(OpenOptions::new().append(true).open(&path)?),
            path,
            write_pos: replay.good_len,
            index: replay.index,
            stale_bytes: replay.stale_bytes,
        })
    }

    /// Sets `key` to `value`, replacing any old value.
    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        let pos = self.append(&Record::Set {
            key: key.clone(),
            value,
        })?;
        if let Some(old) = self.index.insert(key, pos) {
            self.stale_bytes += old.len;
        }
        Ok(())
    }

    /// Returns the value for `key`, if any.
    pub fn get(&mut self, key: &str) -> Result<Option<String>> {
        let Some(&pos) = self.index.get(key) else {
            return Ok(None);
        };
        self.reader.seek(SeekFrom::Start(pos.offset))?;
        let mut buf = Vec::with_capacity(pos.len as usize);
        (&mut self.reader).take(pos.len).read_to_end(&mut buf)?;
        match serde_json::from_slice(&buf)? {
            Record::Set { key: k, value } if k == key => Ok(Some(value)),
            _ => Err(Error::Corrupt(format!(
                "index for {key:?} points at offset {} which is not its set record",
                pos.offset
            ))),
        }
    }

    /// Removes `key`. Fails with [`Error::KeyNotFound`] if it isn't there.
    pub fn remove(&mut self, key: &str) -> Result<()> {
        if !self.index.contains_key(key) {
            return Err(Error::KeyNotFound);
        }
        let pos = self.append(&Record::Rm { key: key.into() })?;
        let old = self.index.remove(key).expect("checked above");
        self.stale_bytes += old.len + pos.len;
        Ok(())
    }

    /// How many keys are stored.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True if the store has no keys.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// True if `key` is in the store.
    pub fn contains_key(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// Bytes in the log that compaction could reclaim.
    pub fn stale_bytes(&self) -> u64 {
        self.stale_bytes
    }

    /// Path of the log file.
    pub fn log_path(&self) -> &Path {
        &self.path
    }

    /// Writes one record and flushes it, returning where it landed.
    fn append(&mut self, record: &Record) -> Result<Pos> {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()?;
        let pos = Pos {
            offset: self.write_pos,
            len: line.len() as u64,
        };
        self.write_pos += pos.len;
        Ok(pos)
    }
}

struct Replay {
    index: HashMap<String, Pos>,
    stale_bytes: u64,
    /// Bytes up to the end of the last complete, valid record.
    good_len: u64,
}

/// Reads the whole log and rebuilds the index.
fn replay(mut reader: impl BufRead) -> Result<Replay> {
    let mut index = HashMap::new();
    let mut stale_bytes = 0;
    let mut offset = 0u64;
    let mut line = Vec::new();

    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)? as u64;
        if n == 0 {
            break;
        }
        let complete = line.ends_with(b"\n");
        let record = if complete {
            serde_json::from_slice::<Record>(&line).ok()
        } else {
            None
        };
        let Some(record) = record else {
            // A bad record is only acceptable as the very last thing in the file.
            let mut rest = Vec::new();
            reader.read_to_end(&mut rest)?;
            if rest.is_empty() {
                break;
            }
            return Err(Error::Corrupt(format!(
                "unreadable record at offset {offset}"
            )));
        };

        let pos = Pos { offset, len: n };
        match record {
            Record::Set { key, .. } => {
                if let Some(old) = index.insert(key, pos) {
                    stale_bytes += old.len;
                }
            }
            Record::Rm { key } => {
                if let Some(old) = index.remove(&key) {
                    stale_bytes += old.len;
                }
                stale_bytes += n;
            }
        }
        offset += n;
    }

    Ok(Replay {
        index,
        stale_bytes,
        good_len: offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn set_get_remove() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("a").unwrap(), None);
        kv.set(s("a"), s("1")).unwrap();
        assert_eq!(kv.get("a").unwrap().as_deref(), Some("1"));
        kv.set(s("a"), s("2")).unwrap();
        assert_eq!(kv.get("a").unwrap().as_deref(), Some("2"));
        kv.remove("a").unwrap();
        assert_eq!(kv.get("a").unwrap(), None);
    }

    #[test]
    fn len_and_contains_key() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert!(kv.is_empty());
        kv.set(s("a"), s("1")).unwrap();
        kv.set(s("b"), s("2")).unwrap();
        kv.set(s("a"), s("3")).unwrap();
        assert_eq!(kv.len(), 2);
        assert!(kv.contains_key("a"));
        assert!(!kv.contains_key("z"));
    }

    #[test]
    fn data_survives_reopening() {
        let dir = TempDir::new().unwrap();
        {
            let mut kv = KvStore::open(dir.path()).unwrap();
            kv.set(s("name"), s("zaid")).unwrap();
            kv.set(s("lang"), s("c++")).unwrap();
            kv.set(s("lang"), s("rust")).unwrap();
            kv.set(s("tmp"), s("x")).unwrap();
            kv.remove("tmp").unwrap();
        }
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("name").unwrap().as_deref(), Some("zaid"));
        assert_eq!(kv.get("lang").unwrap().as_deref(), Some("rust"));
        assert_eq!(kv.get("tmp").unwrap(), None);
        assert_eq!(kv.len(), 2);
    }

    #[test]
    fn removing_a_missing_key_fails() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert!(matches!(kv.remove("nope"), Err(Error::KeyNotFound)));
    }

    #[test]
    fn values_can_hold_anything() {
        let dir = TempDir::new().unwrap();
        let tricky = s("line one\nline \"two\" \u{1F980} {\"op\":\"rm\"}");
        {
            let mut kv = KvStore::open(dir.path()).unwrap();
            kv.set(s("k"), tricky.clone()).unwrap();
        }
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("k").unwrap(), Some(tricky));
    }

    #[test]
    fn half_written_last_record_is_dropped() {
        let dir = TempDir::new().unwrap();
        {
            let mut kv = KvStore::open(dir.path()).unwrap();
            kv.set(s("a"), s("1")).unwrap();
        }
        // Simulate a crash in the middle of writing the next record.
        let log = dir.path().join(LOG_FILE);
        let mut f = OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(br#"{"op":"set","key":"b","val"#).unwrap();
        drop(f);

        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("a").unwrap().as_deref(), Some("1"));
        assert_eq!(kv.get("b").unwrap(), None);
        kv.set(s("c"), s("3")).unwrap();
        drop(kv);
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("c").unwrap().as_deref(), Some("3"));
    }

    #[test]
    fn broken_record_in_the_middle_is_an_error() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(LOG_FILE),
            "{\"op\":\"set\",\"key\":\"a\",\"value\":\"1\"}\nGARBAGE\n{\"op\":\"rm\",\"key\":\"a\"}\n",
        )
        .unwrap();
        assert!(matches!(KvStore::open(dir.path()), Err(Error::Corrupt(_))));
    }

    #[test]
    fn stale_bytes_grow_with_overwrites_and_removes() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        kv.set(s("a"), s("1")).unwrap();
        assert_eq!(kv.stale_bytes(), 0);
        kv.set(s("a"), s("2")).unwrap();
        let after_overwrite = kv.stale_bytes();
        assert!(after_overwrite > 0);
        kv.remove("a").unwrap();
        assert!(kv.stale_bytes() > after_overwrite);
        let total = kv.stale_bytes();
        drop(kv);
        assert_eq!(KvStore::open(dir.path()).unwrap().stale_bytes(), total);
    }
}
