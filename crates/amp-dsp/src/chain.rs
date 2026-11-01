use crate::{Block, MAX_BLOCK};

/// Long enough for the slowest control smoothing to finish.
const SETTLE_SECONDS: f32 = 0.12;

struct Slot {
    block: Box<dyn Block>,
    bypassed: bool,
}

/// An ordered list of blocks. The engine owns one at a time and replaces it
/// whole when the preset or a loaded model changes.
#[derive(Default)]
pub struct Chain {
    slots: Vec<Slot>,
}

impl Chain {
    pub fn push(&mut self, block: Box<dyn Block>, bypassed: bool) {
        self.slots.push(Slot { block, bypassed });
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

    pub fn set_bypassed(&mut self, slot: usize, bypassed: bool) {
        if let Some(slot) = self.slots.get_mut(slot) {
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
    fn silences_non_finite_output() {
        let mut chain = Chain::default();
        chain.push(Box::new(Scale(f32::INFINITY)), false);
        let mut buffer = [1.0, 1.0];
        chain.process(&mut buffer);
        assert_eq!(buffer, [0.0, 0.0]);
    }
}
