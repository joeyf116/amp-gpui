# Build a board

The board is the row of gear in the middle of the window, running from your
guitar on the left to your speakers on the right. Sound passes through the units
in that order. The **Gear** rack on the left holds everything you can add.

## Add, move, and remove gear

- **Add a unit:** drag it from the rack to where you want it. Double-clicking it
  in the rack places it where that kind of gear usually goes.
- **Move a unit:** drag it, or select it and press ++alt+left++ or ++alt+right++.
- **Remove a unit:** select it and press ++delete++, or drag it back to the rack.
- **Bypass a pedal:** press its footswitch, or select it and press ++b++.
- **Tidy up:** **Arrange in usual order** sorts the board into a common order.
  It's a guideline, and you can drag anything back out of it.

The usual order is compression and drives first, then the gate, amp, and cabinet,
then modulation, delay, and reverb.

Double-click a unit, or select it and press ++enter++, to open its settings. Every
edit can be undone with ++ctrl+z++. See [Keyboard and mouse](controls.md) for
all the controls.

## Built-in gear

These all work with no files.

| Unit | What it does | Controls |
| --- | --- | --- |
| Understory amp | An original amp model | Gain, Bass, Mid, Treble, Presence, Master |
| Cabinet | An original closed-back cabinet voice | Level |
| Compressor | Evens out picking dynamics | Threshold, Ratio, Release, Level |
| Drive | A soft-clipping drive for pushing an amp | Drive, Level |
| Noise gate | Cuts hum between notes | Threshold, Release |
| Chorus | One modulated delay line | Rate, Depth, Mix |
| Delay | Repeats of up to 1.5 seconds | Time, Feedback, Mix |
| Reverb | A plate-like reverb | Size, Damping, Mix |

## Add your own amps and pedals

Press **Add .nam** in the rack and choose one or more capture files. Each one is
checked, copied into your library, and listed under **Amp captures** or **Pedal
captures**, depending on what the file says it is. Drag it onto the board like
any other unit.

A capture has three controls:

| Control | What it does |
| --- | --- |
| Input | The level going into the capture, which is usually what changes its gain |
| Output | The level coming out |
| Quality | Trades CPU for detail, on captures that support it |

Captures need to be **48 kHz**, which most are. The app can't resample a capture,
so any other rate gets an error when you put it on the board.

Captures often have their own license, so check the terms of any file before you
share it. The [NAM project](https://www.neuralampmodeler.com/) has tools for
making your own from gear you own.

### NAM A2 captures

NAM A2 files hold several model sizes in one capture. The **Quality** knob moves
between them, so if you get glitches, lower it before you change anything else.
On other captures, it does nothing.

## Use your own cabinet

1. Open the settings of a **Cabinet** on the board.
2. Press **Load cab IR** and choose a `.wav` impulse response.

The file is copied into your library. Stereo files are mixed to mono, the rate
is converted to match the app, and only the first half second is used. Longer
files are mostly room sound and cost CPU on every block.

!!! warning "Watch for two cabinets"

    Some captures already include a cabinet. If yours does, the app warns you, and
    you should remove the **Cabinet** from the board so you don't hear two.

## What makes a good gain structure

- Set **Input** at the top of the window so hard picking is near the top of the
  **In** meter without turning it red.
- Use **Output** at the top of the window for your overall volume. It doesn't
  change your tone.
- On the Understory amp, raising **Gain** adds dirt without much of a jump in
  volume, because the output is trimmed to match. **Master** sets the level.
- Put the noise gate after your drives, so it can catch their hiss.
