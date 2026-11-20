//! A persistent key-value store, built as an append-only log (the Bitcask idea).
//!
//! Every `set` and `rm` is appended to `kvs.log` as one JSON line. Nothing in the
//! file is ever changed in place. In memory we only keep an index from each key to
//! the position of its latest `set` record, so a `get` is one seek and one read.
//!
//! On `open`, the log is replayed from the start to rebuild the index. If the very
//! last record was only half written (a crash in the middle of a write), it is cut
//! off; a broken record anywhere else is reported as corruption.
//!
//! Overwritten and removed values stay in the log as "stale bytes". Compaction
//! copies only the live records into a new file and swaps it in with a rename, so
//! a crash at any point leaves either the old log or the new one, never a mix. It
//! runs on its own once stale bytes pass a threshold and outweigh the live data.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod error;

pub use error::{Error, Result};

/// Name of the log file inside the store's directory.
pub const LOG_FILE: &str = "kvs.log";
/// Temporary file compaction writes before swapping it in.
const COMPACT_FILE: &str = "kvs.log.compact";
/// Default stale-byte level at which compaction runs by itself.
pub const DEFAULT_COMPACTION_THRESHOLD: u64 = 1024 * 1024;

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
    /// the `rm` records themselves). Compaction reclaims them.
    stale_bytes: u64,
    /// Compact automatically once `stale_bytes` reaches this and exceeds the live bytes.
    compaction_threshold: u64,
}

impl KvStore {
    /// Opens the store in `dir`, creating the directory and log if needed.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let path = dir.join(LOG_FILE);
        // A leftover from a compaction that crashed before its rename. The real log
        // was never touched, so the half-built copy can simply go.
        let leftover = dir.join(COMPACT_FILE);
        if leftover.exists() {
            fs::remove_file(&leftover)?;
        }
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
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
        })
    }

    /// Changes when automatic compaction kicks in (see [`DEFAULT_COMPACTION_THRESHOLD`]).
    pub fn set_compaction_threshold(&mut self, bytes: u64) {
        self.compaction_threshold = bytes;
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
        self.maybe_compact()
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
        self.maybe_compact()
    }

    /// Rewrites the log with only the live records.
    ///
    /// 1. Copy each key's latest `set` record into `kvs.log.compact` and sync it.
    /// 2. Point the reader and writer at the new file.
    /// 3. Rename it over `kvs.log`. The rename is atomic, so a crash before it leaves
    ///    the old log untouched (and `open` deletes the half-built copy).
    pub fn compact(&mut self) -> Result<()> {
        let dir = self.path.parent().expect("log path has a parent");
        let tmp_path = dir.join(COMPACT_FILE);
        let mut tmp = BufWriter::new(File::create(&tmp_path)?);

        // Copy in log order, so the old file is read front to back.
        let mut live: Vec<(String, Pos)> =
            self.index.iter().map(|(k, p)| (k.clone(), *p)).collect();
        live.sort_by_key(|(_, pos)| pos.offset);

        let mut new_index = HashMap::with_capacity(live.len());
        let mut offset = 0u64;
        let mut buf = Vec::new();
        for (key, pos) in live {
            self.reader.seek(SeekFrom::Start(pos.offset))?;
            buf.clear();
            (&mut self.reader).take(pos.len).read_to_end(&mut buf)?;
            tmp.write_all(&buf)?;
            new_index.insert(
                key,
                Pos {
                    offset,
                    len: pos.len,
                },
            );
            offset += pos.len;
        }
        tmp.into_inner().map_err(|e| e.into_error())?.sync_all()?;

        // Swap the handles first: on Windows a file can't be replaced while we still
        // hold the old one open for appending.
        self.reader = BufReader::new(File::open(&tmp_path)?);
        self.writer = BufWriter::new(OpenOptions::new().append(true).open(&tmp_path)?);
        fs::rename(&tmp_path, &self.path)?;

        self.index = new_index;
        self.write_pos = offset;
        self.stale_bytes = 0;
        Ok(())
    }

    fn maybe_compact(&mut self) -> Result<()> {
        let live_bytes = self.write_pos - self.stale_bytes;
        if self.stale_bytes >= self.compaction_threshold && self.stale_bytes > live_bytes {
            self.compact()?;
        }
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

    fn log_len(dir: &TempDir) -> u64 {
        fs::metadata(dir.path().join(LOG_FILE)).unwrap().len()
    }

    #[test]
    fn compact_shrinks_the_log_and_keeps_the_data() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        for i in 0..200 {
            kv.set(s("counter"), i.to_string()).unwrap();
        }
        kv.set(s("keep"), s("me")).unwrap();
        kv.set(s("gone"), s("soon")).unwrap();
        kv.remove("gone").unwrap();
        let before = log_len(&dir);

        kv.compact().unwrap();
        assert_eq!(kv.stale_bytes(), 0);
        assert!(log_len(&dir) < before / 10, "log should shrink a lot");
        assert_eq!(kv.get("counter").unwrap().as_deref(), Some("199"));
        assert_eq!(kv.get("keep").unwrap().as_deref(), Some("me"));
        assert_eq!(kv.get("gone").unwrap(), None);

        // Writes after compaction land in the new file and survive reopening.
        kv.set(s("after"), s("compaction")).unwrap();
        drop(kv);
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert_eq!(kv.get("after").unwrap().as_deref(), Some("compaction"));
        assert_eq!(kv.get("counter").unwrap().as_deref(), Some("199"));
        assert_eq!(kv.len(), 3);
        assert!(!dir.path().join(COMPACT_FILE).exists());
    }

    #[test]
    fn compaction_runs_by_itself() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        kv.set_compaction_threshold(2_000);
        for i in 0..1_000 {
            kv.set(s("hot"), i.to_string()).unwrap();
        }
        assert!(kv.stale_bytes() < 2_000, "stale bytes stay bounded");
        assert!(log_len(&dir) < 4_000, "log stays small");
        assert_eq!(kv.get("hot").unwrap().as_deref(), Some("999"));
    }

    #[test]
    fn empty_store_compacts_to_empty() {
        let dir = TempDir::new().unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        kv.set(s("a"), s("1")).unwrap();
        kv.remove("a").unwrap();
        kv.compact().unwrap();
        assert_eq!(log_len(&dir), 0);
        assert!(kv.is_empty());
    }

    #[test]
    fn leftover_compaction_file_is_cleaned_up() {
        let dir = TempDir::new().unwrap();
        {
            let mut kv = KvStore::open(dir.path()).unwrap();
            kv.set(s("a"), s("1")).unwrap();
        }
        // A compaction that crashed before its rename.
        fs::write(dir.path().join(COMPACT_FILE), "half written").unwrap();
        let mut kv = KvStore::open(dir.path()).unwrap();
        assert!(!dir.path().join(COMPACT_FILE).exists());
        assert_eq!(kv.get("a").unwrap().as_deref(), Some("1"));
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
