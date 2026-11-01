# Block reference

Every block's parameters are declared in `BlockKind::params`
(`crates/amp-dsp/src/block.rs`). The `id` column is what presets store, so renaming
an id breaks saved presets. Names and the order of the list can change freely.

## NAM amp and pedal

`kind: "amp"` and `kind: "pedal"` both run `NamAmp` (`amp.rs`). A pedal is the same
thing as an amp except for where it sits, and the gear drawer sorts captures using the
file's `gear_type`.

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `input` | Input | dB | -24 | 24 | 0 |
| `output` | Output | dB | -24 | 24 | 0 |
| `quality` | Quality | % | 0 | 100 | 100 |

`nam-rs` leaves gain staging to its host, so this block adds three things:

- **Normalization.** If the file states its loudness, the output is scaled to
  -18 dBFS RMS, the level NAM's reference plugin uses. Swapping models then doesn't
  change the volume by 20 dB.
- **A DC blocker.** A first-order high-pass at about 5 Hz, after the model.
- **A rate check.** A model fed the wrong rate plays wrong with no error of its own,
  so loading refuses anything but 48 kHz.

With no model loaded, audio passes through untouched. **Quality** sets the slim size
on A2 models, and does nothing on others.

## Understory amp

`kind: "tube"` (`tube.rs`) is an original amp model that needs no capture.

| id | Name | Min | Max | Default |
| --- | --- | --- | --- | --- |
| `gain` | Gain | 0 | 10 | 5 |
| `bass` | Bass | 0 | 10 | 5 |
| `mid` | Mid | 0 | 10 | 5 |
| `treble` | Treble | 0 | 10 | 5 |
| `presence` | Presence | 0 | 10 | 3 |
| `master` | Master | 0 | 10 | 5 |

```mermaid
flowchart LR
    hp[High-pass 95 Hz] --> up[2x oversampling]
    up --> s1[Clipping stage]
    s1 --> s2[Clipping stage]
    s2 --> down[Back to 48 kHz]
    down --> tone[Tone stack]
    tone --> power[Power stage]
```

- **Gain stages:** two `tanh` stages with a small bias, so they clip harder on one
  side. That adds the even harmonics of a real stage. Gain goes up to 46 dB, and
  more gain pushes the bias further off center. Both stages run at double the sample
  rate to hold down aliasing.
- **Tone stack:** bass is a shelf at 140 Hz, mid a peak at 700 Hz, treble a shelf at
  2.6 kHz, and presence a shelf at 4.8 kHz. For the first three, 5 is flat, and each
  step is 2.4 dB.
- **Gain trim:** the output drops 1.2 dB per step of gain, so raising gain adds dirt
  and not a jump in volume.
- **Power stage:** a gentle `tanh` that only rounds off peaks that reach it.

## Cabinet

`kind: "cab"` (`cab.rs`).

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `level` | Level | dB | -24 | 12 | 0 |

With an impulse response, the block does partitioned FFT convolution with 64-sample
partitions. Loading averages the file's channels to mono, resamples it, cuts it to
0.5 seconds, and normalizes its energy.

With no file, it uses a built-in voice: an 80 ms impulse response generated from
filters, so no recording is involved. It has a high-pass at 85 Hz, a bass bump at
115 Hz, a dip at 420 Hz, a peak at 2.5 kHz, and a steep roll-off above about 5 kHz,
which is what tames an amp's fizz.

## Noise gate

`kind: "gate"` (`gate.rs`).

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `threshold` | Threshold | dB | -90 | -20 | -65 |
| `release` | Release | ms | 10 | 500 | 120 |

Once open, the gate stays open down to half the threshold's linear level, so a note
decaying through the threshold doesn't chatter.

## Drive

`kind: "drive"` (`drive.rs`).

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `drive` | Drive | dB | 0 | 36 | 12 |
| `level` | Level | dB | -24 | 6 | -6 |

An asymmetric soft clipper: a biased `tanh`, with the bias subtracted so silence
stays at zero.

## Compressor

`kind: "compressor"` (`effects.rs`). A feed-forward design with a 12 ms attack.

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `threshold` | Threshold | dB | -50 | 0 | -24 |
| `ratio` | Ratio | :1 | 1 | 20 | 3 |
| `release` | Release | ms | 30 | 600 | 160 |
| `level` | Level | dB | -12 | 18 | 3 |

## Chorus

`kind: "chorus"` (`effects.rs`). One delay line, centered on 14 ms, swinging by up
to 6 ms, mixed with the dry signal.

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `rate` | Rate | Hz | 0.1 | 6 | 0.8 |
| `depth` | Depth | % | 0 | 100 | 40 |
| `mix` | Mix | % | 0 | 100 | 40 |

## Delay

`kind: "delay"` (`delay.rs`). The time control is smoothed, so moving it glides and
doesn't click.

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `time` | Time | ms | 20 | 1500 | 380 |
| `feedback` | Feedback | % | 0 | 90 | 30 |
| `mix` | Mix | % | 0 | 100 | 20 |

## Reverb

`kind: "reverb"` (`effects.rs`). Six parallel damped combs feed three all-passes in
series. The comb lengths are prime-spaced to keep echoes from lining up, and a
180 Hz high-pass keeps lows out of the tail so it doesn't turn to mud.

| id | Name | Unit | Min | Max | Default |
| --- | --- | --- | --- | --- | --- |
| `size` | Size | % | 0 | 100 | 50 |
| `damping` | Damping | % | 0 | 100 | 40 |
| `mix` | Mix | % | 0 | 100 | 20 |

## Metronome

`Metronome` (`metronome.rs`) isn't a block either. The processor mixes it in after
the Output level, so turning the rig down or muting it leaves the click alone, and
the safety limiter still catches both. It counts samples with a fractional
remainder, so a tempo that doesn't divide evenly into samples doesn't drift. Each
click is a short decaying sine, higher on the first beat. The UI sends a whole
`Click` at a time, and reads the current beat back from the meters.

## Tuner

The tuner isn't a block. `detect_pitch` (`tuner.rs`) uses the YIN method, which
compares the signal with delayed copies of itself. A plain spectrum won't work,
since the strongest harmonic of a low string is often not its fundamental. It looks
between 55 and 1,400 Hz, over the most recent 6,000 samples, and it runs on the UI
thread on samples the audio thread copies across.
