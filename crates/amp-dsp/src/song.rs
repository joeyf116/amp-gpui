use std::sync::Arc;

/// How much of the way the level moves per sample. About 5 ms, to avoid clicks.
const FADE: f32 = 0.004;
/// Quieter than this, a fade has finished.
const FADED: f32 = 1e-3;

/// A loaded song, mixed in after the amp like the metronome. It counts samples
/// on the audio thread, so its position is what the song really reached.
#[derive(Default)]
pub struct SongPlayer {
    track: Option<Arc<[f32]>>,
    position: usize,
    playing: bool,
    level: f32,
    gain: f32,
    /// Where to go once the song has faded down, so a jump doesn't click.
    seek: Option<usize>,
}

impl SongPlayer {
    /// Swaps in a new track from the start, handing back the old one. The
    /// caller keeps it off the audio thread, where freeing it isn't allowed.
    pub fn load(&mut self, track: Arc<[f32]>) -> Option<Arc<[f32]>> {
        self.position = 0;
        self.playing = false;
        self.gain = 0.0;
        self.seek = None;
        self.track.replace(track)
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn seek(&mut self, position: usize) {
        self.seek = Some(position);
    }

    /// Sets the song's linear gain, independent of the Output level.
    pub fn set_level(&mut self, level: f32) {
        self.level = level;
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn mix(&mut self, buffer: &mut [f32]) {
        let Some(track) = &self.track else {
            return;
        };
        for sample in buffer.iter_mut() {
            let wanted = if self.playing && self.seek.is_none() {
                self.level
            } else {
                0.0
            };
            self.gain += (wanted - self.gain) * FADE;
            if self.gain < FADED
                && let Some(position) = self.seek.take()
            {
                self.position = position;
            }
            let Some(value) = track.get(self.position) else {
                self.playing = false;
                self.gain = 0.0;
                self.seek = None;
                return;
            };
            *sample += value * self.gain;
            // A pause holds its place, but the fade still needs the song moving.
            if self.playing || self.gain >= FADED {
                self.position += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SongPlayer;
    use std::sync::Arc;

    fn player(length: usize) -> SongPlayer {
        let mut player = SongPlayer::default();
        player.load(vec![0.5; length].into());
        player.set_level(1.0);
        player
    }

    #[test]
    fn stays_silent_until_played() {
        let mut player = player(4_096);
        let mut buffer = [0.25; 256];
        player.mix(&mut buffer);
        assert_eq!(buffer, [0.25; 256]);
        assert_eq!(player.position(), 0);
    }

    #[test]
    fn plays_over_what_is_already_there_and_fades_in() {
        let mut player = player(48_000);
        player.set_playing(true);
        let mut buffer = vec![0.25; 8_192];
        player.mix(&mut buffer);
        assert!(buffer[0] < 0.26);
        assert!((buffer[8_191] - 0.75).abs() < 0.01);
        assert_eq!(player.position(), 8_192);
    }

    #[test]
    fn a_seek_fades_down_before_it_jumps() {
        let mut player = player(96_000);
        player.set_playing(true);
        player.mix(&mut vec![0.0; 4_096]);
        player.seek(48_000);
        let mut buffer = vec![0.0; 8_192];
        player.mix(&mut buffer);
        let largest_step = buffer
            .windows(2)
            .fold(0.0_f32, |step, pair| step.max((pair[0] - pair[1]).abs()));
        assert!(largest_step < 0.01, "jumped by {largest_step}");
        assert!(player.position() > 48_000);
    }

    #[test]
    fn stops_at_the_end_of_the_song() {
        let mut player = player(128);
        player.set_playing(true);
        player.mix(&mut [0.0; 512]);
        assert_eq!(player.position(), 128);
        let before = player.position();
        player.mix(&mut [0.0; 512]);
        assert_eq!(player.position(), before);
    }

    #[test]
    fn loading_hands_back_the_old_track() {
        let mut player = player(16);
        let old = player.load(Arc::from(vec![0.0; 8]));
        assert_eq!(old.map(|track| track.len()), Some(16));
    }
}
