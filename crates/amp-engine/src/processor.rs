use amp_dsp::Chain;
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

const COMMAND_CAPACITY: usize = 256;
const RETIRED_CAPACITY: usize = 16;

/// A change from the UI, applied between audio blocks.
pub enum Command {
    SetParam {
        slot: usize,
        param: usize,
        value: f32,
    },
    SetBypassed {
        slot: usize,
        bypassed: bool,
    },
    /// Replaces the whole chain. It is boxed because building one allocates
    /// and loads files, and all of that has to stay off the audio thread.
    SetChain(Box<Chain>),
}

/// Levels and health the audio thread publishes for the UI to poll.
#[derive(Default)]
pub struct Meters {
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    xruns: AtomicU64,
    input_device_us: AtomicU32,
    queue_us: AtomicU32,
    output_device_us: AtomicU32,
}

/// Where the delay between string and speaker goes. Only what the drivers
/// report is counted, so the true total can be higher, especially on Bluetooth.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Latency {
    pub input_device_ms: f32,
    pub queue_ms: f32,
    pub output_device_ms: f32,
}

impl Latency {
    pub fn total_ms(&self) -> f32 {
        self.input_device_ms + self.queue_ms + self.output_device_ms
    }
}

impl Meters {
    pub fn latency(&self) -> Latency {
        let read = |value: &AtomicU32| value.load(Ordering::Relaxed) as f32 / 1_000.0;
        Latency {
            input_device_ms: read(&self.input_device_us),
            queue_ms: read(&self.queue_us),
            output_device_ms: read(&self.output_device_us),
        }
    }

    pub(crate) fn set_input_device_latency(&self, duration: Duration) {
        self.input_device_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    pub(crate) fn set_queue_latency(&self, duration: Duration) {
        self.queue_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    pub(crate) fn set_output_device_latency(&self, duration: Duration) {
        self.output_device_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    /// Reads and clears the peak since the last call, so a short transient
    /// still shows up on a meter drawn 20 times a second.
    pub fn take_input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed))
    }

    pub fn take_output_peak(&self) -> f32 {
        f32::from_bits(self.output_peak.swap(0, Ordering::Relaxed))
    }

    pub fn xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed)
    }

    pub(crate) fn add_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }

    fn raise(peak: &AtomicU32, value: f32) {
        if value > f32::from_bits(peak.load(Ordering::Relaxed)) {
            peak.store(value.to_bits(), Ordering::Relaxed);
        }
    }
}

/// The UI thread's handle on a [`Processor`].
pub struct Controller {
    commands: Producer<Command>,
    retired: Consumer<Box<Chain>>,
    pub meters: Arc<Meters>,
}

impl Controller {
    /// Queues a command, handing it back if the audio side has stopped
    /// draining the queue.
    pub fn send(&mut self, command: Command) -> Result<(), Command> {
        self.commands.push(command).map_err(|error| match error {
            rtrb::PushError::Full(command) => command,
        })
    }

    /// Frees chains the audio thread has finished with. Call this regularly
    /// from the UI thread.
    pub fn drop_retired(&mut self) {
        while self.retired.pop().is_ok() {}
    }
}

/// Runs the chain. Everything in here happens on the audio thread.
pub struct Processor {
    chain: Box<Chain>,
    commands: Consumer<Command>,
    retired: Producer<Box<Chain>>,
    /// A chain waiting for room in `retired`. Dropping it here would free
    /// memory on the audio thread, so it waits until the queue has space.
    pending_retire: Option<Box<Chain>>,
    meters: Arc<Meters>,
}

pub fn processor(chain: Box<Chain>) -> (Processor, Controller) {
    let (command_tx, command_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (retired_tx, retired_rx) = RingBuffer::new(RETIRED_CAPACITY);
    let meters = Arc::new(Meters::default());
    (
        Processor {
            chain,
            commands: command_rx,
            retired: retired_tx,
            pending_retire: None,
            meters: meters.clone(),
        },
        Controller {
            commands: command_tx,
            retired: retired_rx,
            meters,
        },
    )
}

impl Processor {
    pub fn process(&mut self, buffer: &mut [f32]) {
        self.apply_commands();
        Meters::raise(&self.meters.input_peak, peak(buffer));
        self.chain.process(buffer);
        for sample in buffer.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Meters::raise(&self.meters.output_peak, peak(buffer));
    }

    fn apply_commands(&mut self) {
        if let Some(chain) = self.pending_retire.take() {
            self.retire(chain);
        }
        // Only take a new chain once the old one has somewhere to go.
        while self.pending_retire.is_none() {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            match command {
                Command::SetParam { slot, param, value } => {
                    self.chain.set_param(slot, param, value);
                }
                Command::SetBypassed { slot, bypassed } => {
                    self.chain.set_bypassed(slot, bypassed);
                }
                Command::SetChain(chain) => {
                    let old = std::mem::replace(&mut self.chain, chain);
                    self.retire(old);
                }
            }
        }
    }

    fn retire(&mut self, chain: Box<Chain>) {
        if let Err(rtrb::PushError::Full(chain)) = self.retired.push(chain) {
            self.pending_retire = Some(chain);
        }
    }
}

fn peak(buffer: &[f32]) -> f32 {
    buffer
        .iter()
        .fold(0.0, |peak, sample| peak.max(sample.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_dsp::{Block, Chain};

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

    fn scale_chain(gain: f32) -> Box<Chain> {
        let mut chain = Chain::default();
        chain.push(Box::new(Scale(gain)), false);
        Box::new(chain)
    }

    #[test]
    fn applies_params_between_blocks() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(
            controller
                .send(Command::SetParam {
                    slot: 0,
                    param: 0,
                    value: 0.5
                })
                .is_ok()
        );
        let mut buffer = [0.5];
        processor.process(&mut buffer);
        assert_eq!(buffer, [0.25]);
    }

    #[test]
    fn hands_the_old_chain_back_to_the_ui_thread() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(controller.send(Command::SetChain(scale_chain(0.5))).is_ok());
        processor.process(&mut [0.0]);
        assert_eq!(controller.retired.slots(), 1);
        controller.drop_retired();
        assert_eq!(controller.retired.slots(), 0);
    }

    #[test]
    fn holds_a_chain_swap_until_retired_space_frees_up() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        for _ in 0..RETIRED_CAPACITY + 3 {
            assert!(controller.send(Command::SetChain(scale_chain(0.5))).is_ok());
        }
        processor.process(&mut [0.0]);
        assert!(processor.pending_retire.is_some());
        controller.drop_retired();
        processor.process(&mut [0.0]);
        controller.drop_retired();
        processor.process(&mut [0.0]);
        assert!(processor.pending_retire.is_none());
        assert_eq!(processor.commands.slots(), 0);
    }

    #[test]
    fn clamps_output_and_reports_peaks() {
        let (mut processor, controller) = processor(scale_chain(4.0));
        let mut buffer = [0.5, -0.25];
        processor.process(&mut buffer);
        assert_eq!(buffer, [1.0, -1.0]);
        assert_eq!(controller.meters.take_input_peak(), 0.5);
        assert_eq!(controller.meters.take_output_peak(), 1.0);
        assert_eq!(controller.meters.take_output_peak(), 0.0);
    }
}
