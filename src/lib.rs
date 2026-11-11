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
}
