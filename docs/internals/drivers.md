# Audio drivers

Getting from 40 ms to under 10 ms took three ways of opening a device. All three
feed the same `Pump`, so everything above the device layer is identical.

## Measured results

These numbers are driver-reported, measured with the chain muted.

| Route | FLOW 8 to FLOW 8 | FLOW 8 to HDMI speakers |
| --- | --- | --- |
| WASAPI shared | About 40 ms | About 53 ms |
| WASAPI exclusive | About 17 ms | About 18 ms |
| ASIO, 64 frames | 7.7 ms, zero glitches | Not possible |

## WASAPI shared

This is `cpal`'s default on Windows. Audio passes through the Windows mixer, so
other apps keep playing, but the mixer has a 10 ms period that nothing can get
under. The app asks for a 128-frame buffer, though Windows mostly imposes its own
period. This route only accepts 32-bit float.

## WASAPI exclusive

`cpal`'s WASAPI backend is shared mode only, so `crates/amp-engine/src/exclusive.rs`
opens the devices directly through the `windows` crate. The app bypasses the mixer
and talks to each device on its own terms.

- It tries formats in order of preference, starting at 48 kHz, and takes 32-bit
  float through to 16-bit integer samples.
- It asks for the device's minimum period.
- If the device says that size isn't aligned, it reopens with the nearest size the
  device can do, because a failed client can't be reused.
- It runs event-driven, with the device waking the thread each period.
- Its threads register with Windows as "Pro Audio", at critical priority. Without
  that, a short period glitches whenever the machine gets busy.

The cost is that other apps go silent on those devices while monitoring runs.

## ASIO

ASIO talks to the interface's own driver, which is the fastest route there is and
really does honor small buffers. It goes through `cpal` with the `asio` feature,
which is on by default in `amp-app`, at 64 frames.

- Input and output have to be on the one interface, so the device pickers hide.
- Both callbacks run in the same driver pass, at most one buffer apart. The queue
  target starts at one buffer.
- Samples can be 16, 24, or 32-bit integer, or 32-bit float.

Listing drivers means loading each one, which takes a moment. So the scan waits
300 ms after the window opens, and the list fills in late.

### Building with ASIO

The feature needs LLVM, because `cpal` generates its bindings with `libclang`. The
`.cargo/config.toml` file points `LIBCLANG_PATH` at the default install folder.
The first build downloads Steinberg's ASIO SDK.

!!! note "Distribution"

    The app uses the SDK under its GPLv3 option, which is why the app is GPLv3.
    See `packaging/README.md` for what that means for releases.

## Fallback

`Rig::start_monitoring` (`crates/amp-app/src/rig.rs`) builds a list of routes: ASIO
if a driver is chosen, then exclusive unless **Shared audio** is on, then shared.
It tries each in turn and keeps the first error to show if it had to fall back. A
failed start consumes its chain, so each attempt builds a fresh one.

## Testing a device pair

The `device_check` example lists devices. With `--run`, it opens a pair for a few
seconds with the chain muted. Nothing is recorded or audible, so it's safe with a
microphone next to speakers.

```bash
cargo run -p amp-engine --example device_check -- --run --input "Mic 1" --output "USB Out 1/2"
```

<small>![ASIO compatible](../assets/asio-compatible.png){ .asio-logo width="80" }<br>ASIO is a registered trademark of Steinberg Media Technologies GmbH.</small>
