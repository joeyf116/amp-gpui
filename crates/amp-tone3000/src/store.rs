use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

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
    path: PathBuf,
}

impl TokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Anything unreadable counts as signed out, which a new sign-in fixes.
    pub fn load(&self) -> Option<Tokens> {
        let sealed = fs::read(&self.path).ok()?;
        serde_json::from_slice(&seal::open(&sealed).ok()?).ok()
    }

    pub fn save(&self, tokens: &Tokens) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, seal::close(&serde_json::to_vec(tokens)?)?)
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(windows)]
mod seal {
    use std::{io, slice};
    use windows::{
        Win32::{
            Foundation::{HLOCAL, LocalFree},
            Security::Cryptography::{
                CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
            },
        },
        core::PCWSTR,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr().cast_mut(),
        }
    }

    /// Copies out what Windows allocated, then frees it.
    fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: on success Windows hands over a `LocalAlloc` buffer of exactly
        // `cbData` bytes, which is read once here and freed once.
        unsafe {
            let bytes = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(Some(HLOCAL(output.pbData.cast())));
            bytes
        }
    }

    pub fn close(data: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: the input blob borrows `data` for the call, and `output` is a
        // valid out pointer.
        unsafe {
            CryptProtectData(
                &blob(data),
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(io::Error::other)?;
        Ok(take(output))
    }

    pub fn open(data: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `close`.
        unsafe {
            CryptUnprotectData(
                &blob(data),
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(io::Error::other)?;
        Ok(take(output))
    }
}

/// Without a safe place for tokens they aren't written at all, so sign-in lasts
/// until the app closes.
#[cfg(not(windows))]
mod seal {
    use std::io;

    pub fn close(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn open(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

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
        store.clear().unwrap();
    }

    #[test]
    fn a_damaged_file_counts_as_signed_out() {
        let path = std::env::temp_dir().join(format!("amp-t3k-damaged-{}", std::process::id()));
        fs::write(&path, b"not sealed").unwrap();
        assert_eq!(TokenStore::new(path.clone()).load(), None);
        fs::remove_file(path).unwrap();
    }
}
