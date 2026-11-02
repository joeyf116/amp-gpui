//! Parsed captures and decoded IRs, kept between chain builds. Every edit to the
//! board builds a new chain, and without this each build would read and parse
//! every file on it again, which for a `.nam` is megabytes of JSON.

use amp_dsp::NamModel;
use std::sync::Arc;

/// How many of each kind of file to keep. A parsed capture is far smaller than
/// its file, so this covers a busy session without holding a whole library.
const KEEP: usize = 24;

/// An impulse response as read from its file: mono samples and their rate.
pub(crate) type Impulse = Arc<(Vec<f32>, u32)>;

/// Files by content hash, most recently used last.
#[derive(Default)]
pub(crate) struct AssetCache {
    models: Vec<(String, Arc<NamModel>)>,
    impulses: Vec<(String, Impulse)>,
}

fn take<T: Clone>(entries: &mut Vec<(String, T)>, hash: &str) -> Option<T> {
    let index = entries.iter().position(|(key, _)| key == hash)?;
    let entry = entries.remove(index);
    let value = entry.1.clone();
    entries.push(entry);
    Some(value)
}

fn keep<T>(entries: &mut Vec<(String, T)>, hash: &str, value: T) {
    entries.retain(|(key, _)| key != hash);
    entries.push((hash.to_owned(), value));
    if entries.len() > KEEP {
        entries.remove(0);
    }
}

impl AssetCache {
    pub(crate) fn model(&mut self, hash: &str) -> Option<Arc<NamModel>> {
        take(&mut self.models, hash)
    }

    pub(crate) fn keep_model(&mut self, hash: &str, model: Arc<NamModel>) {
        keep(&mut self.models, hash, model);
    }

    pub(crate) fn impulse(&mut self, hash: &str) -> Option<Impulse> {
        take(&mut self.impulses, hash)
    }

    pub(crate) fn keep_impulse(&mut self, hash: &str, impulse: Impulse) {
        keep(&mut self.impulses, hash, impulse);
    }
}

impl std::fmt::Debug for AssetCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AssetCache")
            .field("models", &self.models.len())
            .field("impulses", &self.impulses.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_most_recently_used_files() {
        let mut cache = AssetCache::default();
        let impulse = |rate| Arc::new((vec![1.0], rate));
        for index in 0..KEEP {
            cache.keep_impulse(&index.to_string(), impulse(index as u32));
        }
        // Using the oldest saves it from being the one that goes.
        assert!(cache.impulse("0").is_some());
        cache.keep_impulse("new", impulse(1));
        assert!(cache.impulse("0").is_some());
        assert!(cache.impulse("1").is_none());
        assert_eq!(cache.impulses.len(), KEEP);
    }
}
