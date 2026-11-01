# Presets and the library

A library is a plain folder. Nothing about it depends on the machine that made it,
which is what makes sharing work (`crates/amp-preset/src/library.rs`).

```text
library/
├── presets/          One JSON file per tone
│   └── fern-crunch.json
├── assets/           Captures and IRs, named by hash
│   └── <sha256>.nam
├── practice/         One JSON file per tab sheet
├── captures.json     The rack's catalog
└── factory.json      Which factory tones this library has been given
```

## Preset format

A preset is a list of blocks in chain order. Here's a shortened factory tone:

```json
{
  "schema": 1,
  "name": "Fern Crunch",
  "author": "Amp GPUI",
  "tags": ["crunch"],
  "notes": "Edge of breakup. Dig in to bite, or ease off to clean up.",
  "blocks": [
    {
      "kind": "tube",
      "enabled": true,
      "params": { "gain": 4.6, "master": 4.0 }
    },
    { "kind": "cab", "enabled": true, "params": { "level": 0.0 } }
  ]
}
```

A block that uses a file also has an `asset`:

```json
{
  "kind": "amp",
  "enabled": true,
  "params": { "input": 0.0, "output": 0.0, "quality": 100.0 },
  "asset": {
    "kind": "nam_model",
    "sha256": "<64 hex characters>",
    "name": "My Capture.nam"
  }
}
```

The `kind` values are listed in the [block reference](blocks.md). An asset's `kind`
is `nam_model` or `impulse_response`, and its `name` is for display only.

### Staying compatible

- `params` is keyed by parameter id, so a parameter added later falls back to its
  default on old presets.
- Values are clamped to the parameter's range when read, so a bad file can't push a
  block out of bounds.
- A preset with a newer `schema` than the app is refused, with a message saying so.
- One broken file doesn't hide the rest. The others load, and the app shows the
  error.

## Content addressing

Files are stored by the SHA-256 of their contents, so a preset refers to exactly
one file wherever it goes.

- Importing the same file twice stores it once, whatever it was called.
- Preset files come from other people, so a hash is checked to be 64 hex characters
  before it goes anywhere near a path. Nothing else in a preset ever becomes part of
  a file path. A tone's file name comes from a slug of its name.
- Writes go through a temporary file and a rename, so a crash mid-save can't leave a
  half-written file for a sync tool to spread.

## Tone packs

A pack is a small library holding one preset and the files it needs, so exporting
and importing reuse the same code.

Importing runs in two passes:

1. Every asset is re-hashed and compared with its preset.
2. Only if all of them pass are files copied and presets saved.

A pack with a swapped file therefore leaves the library untouched.

`captures.json` isn't part of a pack. It's a cache of details from each `.nam` file,
kept so the rack doesn't have to parse megabytes of weights to show a list. Tones
from a pack play without it, but their captures don't appear in the rack.

## Factory tones

The factory set is defined in code (`crates/amp-preset/src/factory.rs`) and uses
only built-in blocks. That keeps every factory tone playable with no captures, and
free to share.

`seed_factory_tones` runs at startup, and `factory.json` records which tones the
library has been given. So a tone added in a later version arrives once, and a tone
you deleted stays gone. A library of your own tones with no factory ones in it is
left alone. The tests judge every tone on a synthetic pluck, measuring its level
and checking for clipping.

## Practice sheets

A sheet (`crates/amp-preset/src/practice.rs`) has a title and a list of sections,
each with a name, plain text tab, and an optional tone name. Spacing in tab is
meaningful, so it's never reflowed.

A link is a tone's name, so a renamed tone needs linking again. A hash wouldn't help
here, because the point of the link is to follow a tone as you edit it.

## Settings

Settings are separate from the library, since they're about this machine and not
your tones. `settings.json` holds the chosen devices, input channel, driver, levels,
library location, and last tone.

Set the `AMP_GPUI_DATA` environment variable to use another folder, which keeps test
runs away from a real library.
