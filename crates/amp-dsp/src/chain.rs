use crate::{Block, MAX_BLOCK};

/// Long enough for the slowest control smoothing to finish.
const SETTLE_SECONDS: f32 = 0.12;
/// Below this a fading tail can't be heard, about -80 dB.
const TAIL_FLOOR: f32 = 1e-4;
/// How long a tail has to stay under the floor before it stops running. Longer
/// than the longest delay time, since repeats have silence between them.
const TAIL_QUIET_FRAMES: usize = 96_000;

struct Slot {
    block: Box<dyn Block>,
    bypassed: bool,
    /// Set while a bypassed block's tail is still fading out.
    ringing: bool,
    quiet_frames: usize,
}

/// An ordered list of blocks. The engine owns one at a time and replaces it
/// whole when the preset or a loaded model changes. Settings, switches, and
/// scenes change it in place, so they never interrupt the sound.
#[derive(Default)]
pub struct Chain {
    slots: Vec<Slot>,
    /// Where a bypassed block's tail is run, sized off the audio thread.
    tail: Vec<f32>,
}

impl Chain {
    pub fn push(&mut self, block: Box<dyn Block>, bypassed: bool) {
        if self.tail.is_empty() {
            self.tail = vec![0.0; MAX_BLOCK];
        }
        self.slots.push(Slot {
            block,
            bypassed,
            ringing: false,
            quiet_frames: 0,
        });
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn set_param(&mut self, slot: usize, param: usize, value: f32) {
        if let Some(slot) = self.slots.get_mut(slot) {
            slot.block.set_param(param, value);
        }
    }

    /// Turns a block off or on. A delay or reverb turned off keeps fading
    /// out on its own, with nothing new going in.
    pub fn set_bypassed(&mut self, slot: usize, bypassed: bool) {
        if let Some(slot) = self.slots.get_mut(slot) {
            if bypassed && !slot.bypassed && slot.block.has_tail() {
                slot.ringing = true;
                slot.quiet_frames = 0;
            }
            if !bypassed {
                slot.ringing = false;
            }
            slot.bypassed = bypassed;
        }
    }

    /// Runs `buffer` through every active block. A sample that comes out as
    /// NaN or infinite would stay in every feedback path, so the whole chain is
    /// reset and the buffer silenced instead of passing it to the speakers.
    pub fn process(&mut self, buffer: &mut [f32]) {
        for chunk in buffer.chunks_mut(MAX_BLOCK) {
            for slot in &mut self.slots {
                if !slot.bypassed {
                    slot.block.process(chunk);
                } else if slot.ringing {
                    let tail = &mut self.tail[..chunk.len()];
                    tail.fill(0.0);
                    slot.block.process(tail);
                    let mut loudest = 0.0_f32;
                    for (sample, tail) in chunk.iter_mut().zip(tail.iter()) {
                        *sample += tail;
                        loudest = loudest.max(tail.abs());
                    }
                    slot.quiet_frames = if loudest < TAIL_FLOOR {
                        slot.quiet_frames + chunk.len()
                    } else {
                        0
                    };
                    if slot.quiet_frames > TAIL_QUIET_FRAMES {
                        slot.ringing = false;
                        slot.block.reset();
                    }
                }
            }
            if chunk.iter().any(|sample| !sample.is_finite()) {
                chunk.fill(0.0);
                self.reset();
            }
        }
    }

    /// Runs silence through the chain so every smoothed control reaches its
    /// setting. Without this a new chain starts at unity gain and spends its
    /// first few milliseconds too loud, which is heard as a pop on every edit.
    /// Allocates, so it belongs off the audio thread.
    pub fn settle(&mut self, sample_rate: u32) {
        let mut silence = vec![0.0; (SETTLE_SECONDS * sample_rate as f32) as usize];
        self.process(&mut silence);
    }

    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.block.reset();
            slot.ringing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scale(f32);

    impl Block for Scale {
        fn set_param(&mut self, _: usize, value: f32) {
            self.0 = value;
        }
        fn process(&mut self, buffer: &mut [f32]) {
            buffer.iter_mut().for_each(|sample| *sample *= self.0);
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn runs_blocks_in_order_and_skips_bypassed_ones() {
        let mut chain = Chain::default();
        chain.push(Box::new(Scale(2.0)), false);
        chain.push(Box::new(Scale(10.0)), true);
        chain.push(Box::new(Scale(3.0)), false);
        let mut buffer = [1.0];
        chain.process(&mut buffer);
        assert_eq!(buffer, [6.0]);

        chain.set_bypassed(1, false);
        chain.set_param(0, 0, 1.0);
        let mut buffer = [1.0];
        chain.process(&mut buffer);
        assert_eq!(buffer, [30.0]);
    }

    #[test]
    fn hands_blocks_at_most_max_block_samples() {
        struct Longest<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Block for Longest<'static> {
            fn set_param(&mut self, _: usize, _: f32) {}
            fn process(&mut self, buffer: &mut [f32]) {
                self.0
                    .fetch_max(buffer.len(), std::sync::atomic::Ordering::Relaxed);
            }
            fn reset(&mut self) {}
        }
        static LONGEST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let mut chain = Chain::default();
        chain.push(Box::new(Longest(&LONGEST)), false);
        chain.process(&mut vec![0.0; MAX_BLOCK * 2 + 5]);
        assert_eq!(
            LONGEST.load(std::sync::atomic::Ordering::Relaxed),
            MAX_BLOCK
        );
    }

    #[test]
    fn a_settled_chain_starts_at_its_setting() {
        use crate::{Drive, Gate};
        let build = || {
            let mut drive = Drive::new(48_000);
            drive.set_param(0, 0.0);
            drive.set_param(1, -24.0);
            let mut chain = Chain::default();
            chain.push(Box::new(Gate::new(48_000)), true);
            chain.push(Box::new(drive), false);
            chain
        };
        let mut raw = build();
        let mut settled = build();
        settled.settle(48_000);
        let mut raw_output = [0.5; 8];
        let mut settled_output = [0.5; 8];
        raw.process(&mut raw_output);
        settled.process(&mut settled_output);
        assert!(settled_output[0].abs() < raw_output[0].abs() * 0.5);
    }

    #[test]
    fn a_delay_turned_off_lets_its_repeats_fade_instead_of_cutting_them() {
        use crate::Delay;
        let mut delay = Delay::new(48_000);
        delay.set_param(0, 100.0);
        delay.set_param(1, 50.0);
        delay.set_param(2, 100.0);
        let mut chain = Chain::default();
        chain.push(Box::new(delay), false);
        chain.settle(48_000);
        let mut note = vec![0.0; 2_400];
        note[0] = 1.0;
        chain.process(&mut note);

        chain.set_bypassed(0, true);
        let mut after = vec![0.0; 9_600];
        chain.process(&mut after);
        let loudest = after.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!(
            loudest > 0.3,
            "the first repeat should still sound: {loudest}"
        );

        // New playing passes straight through while the old repeats fade.
        let mut dry = vec![0.25; 64];
        chain.set_bypassed(0, true);
        chain.process(&mut dry);
        assert!(dry.iter().all(|sample| *sample >= 0.25 - 1e-6));

        // Once it has rung out it stops running.
        let mut silence = vec![0.0; TAIL_QUIET_FRAMES * 3];
        chain.process(&mut silence);
        assert!(!chain.slots[0].ringing);
    }

    #[test]
    fn a_block_that_starts_bypassed_has_nothing_to_ring() {
        use crate::Reverb;
        let mut chain = Chain::default();
        chain.push(Box::new(Reverb::new(48_000)), true);
        let mut buffer = [0.5; 8];
        chain.process(&mut buffer);
        assert_eq!(buffer, [0.5; 8]);
    }

    #[test]
    fn silences_non_finite_output() {
        let mut chain = Chain::default();
        chain.push(Box::new(Scale(f32::INFINITY)), false);
        let mut buffer = [1.0, 1.0];
        chain.process(&mut buffer);
        assert_eq!(buffer, [0.0, 0.0]);
    }
}
