# Interface and art

The window has four bands, and the code for each lives under
`crates/amp-app/src/rig/`.

| Band | What it holds | Code |
| --- | --- | --- |
| Header | Logo, tone picker, Save, Undo, Redo, levels, meters, Monitor | `header.rs` |
| Chain strip | The units in signal order, with drag-and-drop | `board.rs` |
| Stage | The selected unit up close, its details, and the gear drawer | `board.rs` |
| Footer | Audio status, Audio setup, Tuner, and view tabs | `audio.rs` |

The Tone Explorer (`explorer.rs`) and Audio setup (`audio.rs`) open over the stage.
Which one is open is held in `Overlay`, and ++esc++ closes it. Monitor stays in the
header because live playing is what the app is for. Devices fold away because they
are set once.

The stage scales each unit to fit the room left in the window, down to about half
size, so nothing clips on a small window.

## Tone shelves

The shelves are filtered in `visible`, which has tests of its own. Tags come from the
tones themselves. Favorites and recent tones are stored by name in `settings.json`,
not in the tone files, so sharing a tone doesn't share your favorites. A renamed tone
leaves those lists.

## How the gear art is made

`tools/gear-art/render.py` renders every picture from scratch, with no photos or
traced shapes. Each surface is a height field: tolex, paint, knurling, and chrome are
all bumps in a grid of numbers. The script lights that grid from the top left, which
gives every piece matching highlights and shadows.

```bash
python tools/gear-art/render.py
```

The output goes to `crates/amp-app/assets/gear/` and is compiled into the app, with a
full-size picture for the stage and a small one for the strip. It needs numpy and
Pillow, and the fixed seed makes reruns repeatable. The same script writes the logo
and the forest backdrop.

Pictures leave out anything that changes. `art.rs` lays these over them:

- **Knobs:** a fixed picture of the knob, keeping its light in place, with a pointer
  and printed scale painted on top (`knob.rs`).
- **Lamps:** drawn only while a unit is on.
- **Names:** printed live, so a capture's name lands on its nameplate. They use
  Bahnschrift, which ships with Windows, and fall back to the UI font without it.

Printed names that look like a product are only ever a capture's own file name. A
cabinet with a loaded IR swaps to a picture with bare speakers.

## What keeps it original

The layout borrows ideas common to amp plugins: a visible signal chain, one large
section at a time, and a tone browser with search and tags. The look is our own.
The chain stays free to reorder, the logo is tree rings cut by a waveform, and the
gear shows generic types: a tube head, a 4x12, and stompboxes. When you add art, keep
to that. Don't copy another company's logos, names, graphics, or product designs, or
the color and shape pairings that identify a real pedal.
