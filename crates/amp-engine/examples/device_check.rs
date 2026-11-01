//! Lists audio devices, and with `--run` opens a pair for a few seconds with
//! the chain muted. Nothing is recorded and nothing is audible, so it is safe
//! with a microphone next to speakers.
//!
//! cargo run -p amp-engine --example device_check -- --run --input "Mic 1" --output "USB Out 1/2"

use amp_dsp::{Block, Chain};
use amp_engine::{DeviceList, DeviceOption, Engine, EngineConfig, LatencyMode};
use std::time::Duration;

struct Mute;

impl Block for Mute {
    fn set_param(&mut self, _: usize, _: f32) {}
    fn process(&mut self, buffer: &mut [f32]) {
        buffer.fill(0.0);
    }
    fn reset(&mut self) {}
}

fn pick(devices: &[DeviceOption], flag: &str) -> Option<String> {
    let arguments: Vec<String> = std::env::args().collect();
    let name = arguments
        .iter()
        .position(|a| a == flag)
        .map(|i| &arguments[i + 1])?;
    let device = devices
        .iter()
        .find(|device| device.name.contains(name.as_str()));
    Some(
        device
            .unwrap_or_else(|| panic!("no device matches {name}"))
            .id
            .clone(),
    )
}

fn option(flag: &str) -> Option<String> {
    let arguments: Vec<String> = std::env::args().collect();
    let index = arguments.iter().position(|argument| argument == flag)?;
    arguments.get(index + 1).cloned()
}

fn mode() -> LatencyMode {
    #[cfg(feature = "asio")]
    if option("--asio").is_some() {
        return LatencyMode::Asio;
    }
    if std::env::args().any(|argument| argument == "--exclusive") {
        LatencyMode::Exclusive
    } else {
        LatencyMode::Shared
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "asio")]
    if std::env::args().any(|argument| argument == "--list-asio") {
        let started = std::time::Instant::now();
        println!("{:?} in {:?}", Engine::asio_devices(), started.elapsed());
        return Ok(());
    }
    let devices = DeviceList::scan()?;
    println!(
        "{} inputs, {} outputs",
        devices.inputs.len(),
        devices.outputs.len()
    );

    if std::env::args().any(|argument| argument == "--run") {
        let mut chain = Chain::default();
        chain.push(Box::new(Mute), false);
        let config = EngineConfig {
            input: pick(&devices.inputs, "--input"),
            output: pick(&devices.outputs, "--output"),
            input_channel: 0,
            mode: mode(),
            asio_driver: option("--asio"),
        };
        let engine = Engine::start(Box::new(chain), &config)?;
        let info = engine.info();
        println!(
            "running: {} ({} Hz) -> {} ({} Hz)",
            info.input_name, info.input_rate, info.output_name, info.output_rate
        );
        for _ in 0..6 {
            std::thread::sleep(Duration::from_secs(1));
            let latency = engine.meters().latency();
            println!(
                "total {:.1} ms = input {:.1} + queue {:.1} + output {:.1}, glitches {}",
                latency.total_ms(),
                latency.input_device_ms,
                latency.queue_ms,
                latency.output_device_ms,
                engine.meters().xruns()
            );
        }
    }
    Ok(())
}
