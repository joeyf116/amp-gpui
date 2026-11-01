# Amp GPUI

A desktop guitar amp and effects rig written in Rust. Plug a guitar into an
audio interface, load an amp tone, and play through your speakers, with no
pedal or modeler box in between.

Amps are [Neural Amp Modeler](https://www.neuralampmodeler.com/) captures
(`.nam`), cabinets are impulse responses (`.wav`), and tones live in a plain
folder you own and can share however you like. NAM A2 captures hold several
model sizes, and the amp's Quality knob trades CPU for detail between them.

## Run it

```bash
cargo run -p amp-app
```

On Windows, build from PowerShell. Git Bash puts its own `link.exe` ahead of
the MSVC one.

Then pick your interface under **Guitar in**, your speakers under **Speakers
out**, load a `.nam` capture, and press **Start monitoring**. Turn off your
interface's own direct monitoring so you only hear the processed signal.

To check a device pair without making a sound:

```bash
cargo run -p amp-engine --example device_check -- --run --input "Mic 1" --output "USB Out 1/2"
```

## Layout

| Crate | Job | Runs on |
| --- | --- | --- |
| `amp-dsp` | Blocks and the chain: gate, drive, NAM amp, IR cab, delay | Audio thread |
| `amp-engine` | Devices, resampling, clock drift, UI to audio messaging | Audio thread and UI thread |
| `amp-preset` | Preset format, tone library, tone packs | Background threads |
| `amp-app` | The GPUI interface | UI thread |

## How the audio path works

```text
interface -> queue -> resample to 48 kHz -> chain -> resample to device -> speakers
```

- The input and output can be separate devices with separate clocks, because
  interface in and computer speakers out is the main use case. A cubic
  resampler adapts to differing rates, and the input ratio is continually
  nudged to keep the queue between the devices at a fixed depth.
- The chain always runs at 48 kHz, in 64-frame blocks.
- Nothing on the audio thread allocates, locks, or blocks. The UI sends changes
  through a lock-free queue, new chains are built in the background and swapped
  in whole, and old chains go back to the UI thread to be freed.

## Tone library

A library is a folder with `presets/*.json` and `assets/<sha256>.<nam|wav>`.
Because a preset refers to files by content hash, it means the same thing
wherever it goes. Put the folder in git, a synced drive, or a network share to
share a whole library. Export writes a tone pack, which is the same layout with
one preset. Importing a pack re-hashes every file before accepting any of it.

`.nam` and `.wav` files are ignored by this repo, since captures often have
their own licenses. Check a capture's terms before sharing it in a pack.

## Status

Latency is the main open problem. See the architecture notes for the plan.
