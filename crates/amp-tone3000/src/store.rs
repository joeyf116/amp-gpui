use amp_secret::SealedFile;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

/// A signed-in session. Refresh tokens rotate, so every new pair has to be
/// written down or the session is lost.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
}

/// Keeps tokens on disk, encrypted to the Windows account.
#[derive(Clone, Debug)]
pub(crate) struct TokenStore {
    file: SealedFile,
}

impl TokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            file: SealedFile::new(path),
        }
    }

    /// Anything unreadable counts as signed out, which a new sign-in fixes.
    pub fn load(&self) -> Option<Tokens> {
        serde_json::from_slice(&self.file.load()?).ok()
    }

    pub fn save(&self, tokens: &Tokens) -> io::Result<()> {
        self.file.save(&serde_json::to_vec(tokens)?)
    }

    pub fn clear(&self) -> io::Result<()> {
        self.file.clear()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn tokens_round_trip_without_being_readable_on_disk() {
        let path = std::env::temp_dir().join(format!("amp-t3k-store-{}", std::process::id()));
        let store = TokenStore::new(path.clone());
        let tokens = Tokens {
            access_token: "access-marker".into(),
            refresh_token: "refresh-marker".into(),
            expires_at: 99,
        };
        store.save(&tokens).unwrap();
        assert!(
            !fs::read(&path)
                .unwrap()
                .windows(6)
                .any(|bytes| bytes == b"marker")
        );
        assert_eq!(store.load(), Some(tokens));
        store.clear().unwrap();
        assert_eq!(store.load(), None);
    }
}
