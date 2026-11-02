# Presets and the library

A library is a plain folder. Nothing about it depends on the machine that made it,
which is what makes sharing work (`crates/amp-preset/src/library.rs`).

```text
library/
├── presets/          One JSON file per tone
│   └── fern-crunch.json
├── assets/           Captures and IRs, named by hash
│   └── <sha256>.nam
├── captures.json     The gear drawer's catalog
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

A tone with [scenes](../guide/scenes.md) also has `scenes`, `scene`, and
`level_db`:

```json
{
  "schema": 1,
  "name": "Verse and Solo",
  "blocks": [
    { "kind": "drive", "enabled": true, "params": { "drive": 18.0, "level": -4.0 } },
    { "kind": "tube", "enabled": true, "params": { "gain": 4.6, "master": 4.0 } }
  ],
  "level_db": 3.0,
  "scenes": [
    {
      "level_db": -1.5,
      "blocks": [
        { "enabled": false, "params": { "drive": 12.0, "level": -6.0 } },
        { "enabled": true, "params": { "gain": 4.6, "master": 4.0 } }
      ]
    },
    {
      "level_db": 3.0,
      "blocks": [
        { "enabled": true, "params": { "drive": 18.0, "level": -4.0 } },
        { "enabled": true, "params": { "gain": 4.6, "master": 4.0 } }
      ]
    }
  ],
  "scene": 1
}
```

- `blocks` and `level_db` always hold the scene that's playing. A build of the app
  from before scenes ignores the other fields and plays that scene.
- A scene stores each block's `enabled` and `params`, one entry per block in chain
  order. The chain itself, and each block's file, are shared by every scene.
- `Preset::insert_block`, `remove_block`, `move_block`, `reorder`, and
  `replace_block` keep scenes in line with the chain. Anything that edits `blocks`
  directly has to use them.
- A preset from a file goes through `align_scenes` when it's read. A scene that
  doesn't have one entry per block is reset to the playing scene, keeping its name
  and level, and a lone scene is dropped.
- `sounds_like` compares two presets whichever scene each is showing. The unsaved
  mark uses it, so switching scenes isn't an edit.
- When the assistant edits the loaded tone, `with_chain` gives the playing scene the
  suggestion. If the chain still lines up kind by kind, the other scenes keep their
  settings; otherwise they all start again from the suggestion.

The `kind` values are listed in the [block reference](blocks.md). An asset's `kind`
is `nam_model` or `impulse_response`, and its `name` is for display only.

### Staying compatible

- `params` is keyed by parameter id, so a parameter added later falls back to its
  default on old presets.
- Values are clamped to the parameter's range when read, so a bad file can't push a
  block out of bounds.
- A preset with a newer `schema` than the app is refused, with a message saying so.
- New fields are optional and leave `schema` at 1. A plain tone saves without
  them, so its file is the same as before.
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
kept so the gear drawer doesn't have to parse megabytes of weights to show a list. Tones
from a pack play without it, but their captures don't appear in the drawer.

## Generated tones

The [tone generator](../guide/generate.md) runs in the browser, in
`docs/assets/generator`, with no server of ours behind it:

- `catalog.js` holds every block's parameter ranges and the factory tones. It's
  generated by `tools/tone-generator/sync_catalog.py`, and the docs workflow fails if
  it falls behind `crates/amp-dsp/src/block.rs`.
- `core.js` builds the prompt, checks Claude's reply, and writes the tone pack. The
  factory tones go in the prompt as examples, which is how Claude gets its levels.
- `ui.js` is the interface. It asks Claude through the artifact runtime when it runs
  on claude.ai, and offers a copied prompt or an API key on the docs site.
- `anthropic-sdk.js` is a self-hosted bundle of the Anthropic SDK, so a page that
  handles API keys loads no code from another host. To update it, change the version
  in `tools/tone-generator/package.json` and run `npm install` and `npm run bundle`
  there.

A reply is treated like any other file from someone else. `normalize` keeps only
known block kinds and parameter ids, clamps every value, and drops anything else,
including `asset`, so a generated preset can't refer to a file.

The artifact is one HTML file built from these same sources by
`tools/tone-generator/build_artifact.py`. Publish that file again after changing
them, because the docs deploy doesn't update it.

## Factory tones

The factory set is defined in code (`crates/amp-preset/src/factory.rs`) and uses
only built-in blocks. That keeps every factory tone playable with no captures, and
free to share.

`seed_factory_tones` runs at startup, and `factory.json` records which tones the
library has been given. So a tone added in a later version arrives once, and a tone
you deleted stays gone. A library of your own tones with no factory ones in it is
left alone. The tests judge every tone on a synthetic pluck, measuring its level
and checking for clipping.


## Settings

Settings are separate from the library, since they're about this machine and not
your tones. `settings.json` holds the chosen devices, input channel, driver, levels,
library location, and last tone.

Set the `AMP_GPUI_DATA` environment variable to use another folder, which keeps test
runs away from a real library.
