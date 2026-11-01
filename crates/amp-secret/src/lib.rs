//! Small secrets on disk, such as sign-in tokens and API keys. On Windows they
//! are encrypted to the account with DPAPI, so another account on the PC, or a
//! copy of the file, can't read them. Elsewhere nothing is written at all, and
//! the secret lasts until the app closes.

use std::{fs, io, path::PathBuf};

/// A file holding one encrypted value.
#[derive(Clone, Debug)]
pub struct SealedFile {
    path: PathBuf,
}

impl SealedFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Anything unreadable counts as nothing stored.
    pub fn load(&self) -> Option<Vec<u8>> {
        seal::open(&fs::read(&self.path).ok()?).ok()
    }

    pub fn save(&self, secret: &[u8]) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, seal::close(secret)?)
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
    fn a_secret_round_trips_without_being_readable_on_disk() {
        let path = std::env::temp_dir().join(format!("amp-secret-{}", std::process::id()));
        let file = SealedFile::new(path.clone());
        file.save(b"some-marker").unwrap();
        assert!(
            !fs::read(&path)
                .unwrap()
                .windows(6)
                .any(|bytes| bytes == b"marker")
        );
        assert_eq!(file.load().as_deref(), Some(&b"some-marker"[..]));
        file.clear().unwrap();
        assert_eq!(file.load(), None);
        file.clear().unwrap();
    }

    #[test]
    fn a_damaged_file_counts_as_empty() {
        let path = std::env::temp_dir().join(format!("amp-secret-bad-{}", std::process::id()));
        fs::write(&path, b"not sealed").unwrap();
        assert_eq!(SealedFile::new(path.clone()).load(), None);
        fs::remove_file(path).unwrap();
    }
}
