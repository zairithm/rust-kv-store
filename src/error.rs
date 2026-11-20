use std::fmt;
use std::io;

/// Everything that can go wrong in the store.
#[derive(Debug)]
pub enum Error {
    /// Reading or writing the log failed.
    Io(io::Error),
    /// A record in the log could not be encoded or decoded.
    Serde(serde_json::Error),
    /// `remove` was called for a key that isn't there.
    KeyNotFound,
    /// The index pointed at a record that isn't a `set` for that key. Means the
    /// log was changed by something else while the store was open.
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Serde(e) => write!(f, "bad record: {e}"),
            Self::KeyNotFound => write!(f, "Key not found"),
            Self::Corrupt(msg) => write!(f, "corrupt log: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Serde(e) => Some(e),
            Self::KeyNotFound | Self::Corrupt(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(e)
    }
}
