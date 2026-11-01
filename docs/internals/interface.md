# Interface and art

The window has four bands, and the code for each lives under
`crates/amp-app/src/rig/`.

| Band | What it holds | Code |
| --- | --- | --- |
| Header | Logo, tone picker, Save, Undo, Redo, levels, meters, Monitor | `header.rs` |
| Chain strip | The units in signal order, with drag-and-drop | `board.rs` |
| Stage | The selected unit up close, its details, and the gear drawer | `board.rs` |
| Footer | Audio status, Audio setup, tempo, and Practice tools | `audio.rs` |

Keys are in `keys.rs`, and loading, saving, and undo are in `tones.rs`. The tone
assistant (`assistant.rs`, drawn by `assistant_view.rs`) docks to the right of all of
this, so it stays open beside any panel. The Tone Explorer (`explorer.rs`), the TONE3000
panel (`tone3000.rs`), and Audio setup (`audio.rs`) open over the stage.
Which one is open is held in `Overlay`, and ++esc++ closes it. Monitor stays in the
header because live playing is what the app is for. Devices fold away because they
are set once.

The stage scales each unit to fit the room left in the window, down to about half
size, so nothing clips on a small window.

## Color

Neutrals come first: near-black surfaces and near-white text, each carrying a trace
of green so they don't feel cold. Color is reserved for meaning, and it comes from
the logo.

| Role | Value |
| --- | --- |
| Window | `#0f1110` |
| Panels | `#171918` |
| Raised and hover | `#1f2221` |
| Text | `#eef1ef` |
| Secondary text | `#a3a8a5` |
| Lines | White at about 12% |
| Accent, for what's live or selected | `#7fd6a0`, the logo's rings |
| Highlight | `#e7f2ea`, the logo's waveform |

Amber, red, and blue stay for warnings, errors, and notices. Gear is muted too, so
pedals differ by shade, finish, and printed graphic. The exception is a tone with its
own picture, which shows on its unit as a label (`art::stage`). Tube glow stays warm,
since that's the color of a real tube. The values live in
`crates/amp-app/assets/theme.json` and `style.rs`, and `docs/stylesheets/forest.css`
carries them to this site.

## The Tone Explorer

The list is short, so the Explorer leaves out what a large shop needs. An earlier version
had a filter sidebar, per-option counts, and a row of applied chips, and most of that
explained itself rather than helping anyone find a tone. Now each row of pills picks one
thing. A lit pill is the applied filter, so there's no AND or OR to learn. Options that
would leave nothing are hidden, and rarer filters fold behind **More**.

`visible` does the filtering and has tests of its own, and `choose` keeps one pick per
group. Tags sort into groups by `group_of`: known gain words are the gain row, `rhythm`
and `lead` are roles, and anything else is a style, so a tag from a shared tone still has
a place. Favorites and recent tones are stored by name in `settings.json`, not in the
tone files, so sharing a tone doesn't share your favorites.

## The tone assistant

`crates/amp-assistant` holds everything but the UI, so its tests need no window. Rust
has no official Anthropic SDK, so `api.rs` speaks the Messages API over HTTP and parses
the event stream itself. `AMP_GPUI_ANTHROPIC` points it at a test server, and only a
loopback address counts, so a stray variable can't send the key elsewhere.

A turn runs on its own thread. It gets a `Snapshot` of the rig, so it shares nothing with
the UI thread, and reports through a channel that the meter tick drains. That's why Stop
works at once whatever the network is doing: the UI stops listening, and the thread's
later events go nowhere.

Claude's blocks are kept exactly as they arrive and sent back unchanged, because thinking
blocks are only valid that way. Regenerate and Edit cut the end off, which leaves the rest
untouched. Saved chats keep only words and suggestions, which is why a reopened chat
restarts from plain text.

The rules that keep it safe, which should stay true when you change it:

- **Tools don't touch the rig.** Reading tools run freely. `suggest_tone` only produces a
  `Proposal`, checked by `build_proposal`, and a bad value goes back to Claude as a tool
  error.
- **Downloads follow a press.** `try_proposal` does the downloading, because TONE3000's
  terms allow downloads only when a user asks. The assistant reads only the free tier's
  bounded lists and never calls search, and anything else goes through the Select flow.
- **Text from tones is data.** `remember` is limited to what the player said, and it shows
  in the chat, because tone descriptions are written by strangers.
- **Levels are measured.** `amp_preset::loudness` runs a test chord through the chain, and
  `trim_to` turns the cab or capture output down if it's over 2 dB louder than what was
  playing. It never turns anything up.
- **The block reference is generated.** `prompt.rs` builds it from `BlockKind::params`, so
  a new block or param reaches Claude by itself, once it has a note in `BLOCK_NOTES`.

`taste.rs` holds what's been learned. Adjustments are measured against the tone as it went
on the rig, after any trim, so only the player's own moves count, as a share of each
knob's range. A move has to repeat before it becomes a habit Claude hears about.

The prompt's long first block stays byte for byte the same, so it can be cached. Taste and
the rig come after it. Requests use Claude Opus 5.5 unless the player picks another model,
at medium effort, with server-side fallback on the models that support it.

## Practice tools

`tools.rs` holds the tuner and metronome, and `fretboard.rs` the neck. The tuner only
listens while the panel is open: `sync_tuner` follows the overlay on each meter tick,
however it was closed. The needle eases toward each reading so it doesn't jump.

`Hearing` in `fretboard.rs` keeps no UI state, so its tests can feed it sound. It takes what `Spectrum` in `crates/amp-dsp/src/listen.rs`
hears. `Spectrum::notes` finds several notes by iterative harmonic cancellation: it
takes the note whose harmonics best explain the spectrum, removes what that note
accounts for, and repeats. Each partial's removal is capped by its neighbours, so a
partial two notes share isn't all taken by the first. `chord_name` matches the result
against chord shapes, and a chord has to hold for two looks before it shows.

`song.rs` holds a loaded song. `read_song` in `crates/amp-engine/src/song_file.rs`
decodes the file to mono at the engine rate with Symphonia, and `transcribe` in
`crates/amp-dsp/src/melody.rs` finds the melody on a background thread. It works at
12 kHz with a 10 ms hop. Each frame's spectral peaks vote for the pitches they could be
harmonics of, a Viterbi pass follows the strongest smooth path with a silent state for
gaps, and the path becomes notes after taking out the recording's own tuning. It's a
reimplementation of the salience and tracking idea, with no model or extra data.
`finger_line` then chooses frets by dynamic programming over the whole line, since unlike
live notes it can see what's coming.

Playback is `SongPlayer` in `crates/amp-dsp/src/song.rs`, mixed in after the Output level
like the metronome. The track is an `Arc<[f32]>`: the UI keeps a clone to reload a
restarted engine, and a replaced track goes back through a retired queue so the audio
thread never frees it. The audio thread publishes its sample position in `Meters`, and
the tab and neck draw from that, so they follow what's actually playing.

A pitch can't say where it was fretted, so `place_note` scores each place by hand
distance and by stiffness. Stiffness is the inharmonicity B in
`partial(n) = n * f0 * sqrt(1 + B * n * n)`, fitted from the exact partial frequencies
(`Spectrum::stiffness`). It grows by `2^(fret / 6)` as a string gets shorter, so one
open-string reading predicts every fret, which is how **Learn my guitar** works. That
idea comes from published work on string classification, which reports about 83 to 85
percent from audio alone and 91 to 93 percent with playability added.
`place_chord` searches for the shape with one note per string, in order, within a
four-fret stretch.

`crates/amp-dsp/tests/listen.rs` measures all this on modeled plucks. Run it with
`--nocapture` to see the numbers. Testing on recordings of real guitars would be the
next step, since the model can't show everything a pickup does.

## TONE3000

`crates/amp-tone3000` is the API client, and `rig/tone3000.rs` is its UI. Sign-in is
OAuth with PKCE in the system browser, returning to a listener on
`http://127.0.0.1:53710/callback`. The app holds only the publishable key. The secret
key is for servers and must never enter this repo. Tokens are sealed with DPAPI in
`tone3000.tokens` beside `settings.json`. Refresh tokens rotate, so each new pair is
saved at once. `AMP_GPUI_TONE3000` points the client at a test server. `amp-secret`
does the sealing, and the assistant's API key uses it too.

Keep to these rules when changing it, because access can be revoked without them:

- The [design requirements](https://www.tone3000.com/api#design-requirements) set
  what each screen shows: the splash before sign-in, the fields on tone lists and
  details, the marked chain block, and the full logo appearing before the T3K mark.
- The [API terms](https://www.tone3000.com/api/terms) bar bulk downloads, caching the
  catalog, and bundling or passing on its files, and require keeping creator credit,
  so a model downloads only when someone picks it, and `ToneOrigin` travels with it.
- The free tier covers the Select flow and the bounded lists, so there's no search
  or catalog browser of our own. Selling the app needs a commercial agreement first.

The logos in `assets/tone3000/` are TONE3000's own files, used as they ship. They
are the one exception to the rule below about other companies' logos.

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
