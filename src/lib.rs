//! A key-value store. It starts in memory; the issues take it to a
//! log-structured store on disk with compaction and a network server.

use std::collections::HashMap;

/// An in-memory string key-value store.
#[derive(Debug, Default)]
pub struct KvStore {
    map: HashMap<String, String>,
}

impl KvStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key` to `value`, replacing any old value.
    pub fn set(&mut self, key: String, value: String) {
        self.map.insert(key, value);
    }

    /// Returns the value for `key`, if any.
    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    /// Removes `key`, returning its old value.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }

    // ---------- NAYA: yahan se ----------

    /// Returns how many keys are stored.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if the store has no keys.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns `true` if `key` is in the store.
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    // ---------- NAYA: yahan tak ----------
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove() {
        let mut kv = KvStore::new();
        assert_eq!(kv.get("a"), None);
        kv.set("a".into(), "1".into());
        assert_eq!(kv.get("a").as_deref(), Some("1"));
        kv.set("a".into(), "2".into());
        assert_eq!(kv.get("a").as_deref(), Some("2"));
        assert_eq!(kv.remove("a").as_deref(), Some("2"));
        assert_eq!(kv.get("a"), None);
    }

    // ---------- NAYA: tests ----------

    #[test]
    fn len_counts_keys() {
        let mut kv = KvStore::new();
        assert_eq!(kv.len(), 0);
        assert!(kv.is_empty());

        kv.set("a".into(), "1".into());
        kv.set("b".into(), "2".into());
        assert_eq!(kv.len(), 2);
        assert!(!kv.is_empty());
    }

    #[test]
    fn contains_key_works() {
        let mut kv = KvStore::new();
        kv.set("a".into(), "1".into());
        assert!(kv.contains_key("a"));
        assert!(!kv.contains_key("z"));
    }
}
