# Load and save tones

A tone is a saved rig: every unit in the chain, in order, with its settings. The
tone name in the header shows what's loaded, and an asterisk means it has unsaved
edits.

## Load a tone

Press the arrows beside the name, or ++page-up++ and ++page-down++, to step through
your tones.

For everything else, click the name or press ++ctrl+o++ to open the **Tone
Explorer**. It opens ready for typing.

![The Tone Explorer](../assets/screens/tone-explorer.webp)

- **Find:** type in the search box. It matches names, tags, notes, and the gear in a
  tone, so the name of a capture finds the tones that use it.
- **Filter:** the first row of pills picks a shelf, such as **Favorites** or **My
  tones**. The second picks a gain, from **Clean** to **Fuzz**. **More** shows role and
  style. Each row picks one thing, a lit pill is on, and pressing it again turns it off.
  Options with nothing to show stay hidden.
- **Compare:** click a tone, or press ++up++ and ++down++, to load it while the
  Explorer stays open, so you can try several as you play.
- **Read about it:** the right side describes the loaded tone and lists its signal
  chain.
- **Favorites:** press the star on a tone, or the star in the header.

If nothing matches, press **Ask the assistant** to carry your search over to the
[tone assistant](assistant.md).

Double-click a tone, or press ++esc++, to close.

## Factory tones

A new library starts with these 14 tones. They use only built-in blocks, so
they need no captures or impulse responses.

| Tone | Style | Sound |
| --- | --- | --- |
| Moss Clean | Clean | A warm, even clean for chords and fingerstyle |
| Glass Creek | Clean, chorus | Bright clean with a slow shimmer and a short slap |
| Silver Hollow | Clean, blues | A glassy, round clean with light compression and a plate-style reverb |
| Sunlit Neo | Clean, neo-soul | A shimmering clean for chord melodies and fills |
| Velvet Pocket | Clean, neo-soul, rhythm | A tight, percussive clean for funky rhythm and double-stops |
| Slow Thaw | Blues, edge of breakup | Warm edge of breakup, with a light drive left on as a gentle push |
| Fern Crunch | Crunch | Edge of breakup. Dig in to bite, or ease off to clean up |
| Basalt Drive | Crunch, rhythm | Medium gain with weight, for big open chords and riffs |
| Bark Rhythm | High gain, rhythm | Tight palm mutes, with a drive trimming the lows going into the amp |
| Peat Lead | High gain, lead | Singing mids for solos, with a delay trailing behind the notes |
| Amber Lead | Blues, lead | A vocal lead with a mid push, for long bends over a soft delay |
| Dew Line | Neo-soul, lead | A singing, airy lead with a light chorus and a lush tail |
| Ember Fuzz | Fuzz | A woolly wall of sustain, driving a mostly clean amp |
| Hollow Fog | Ambient | Long washes for swells and slow chords |

!!! note

    The factory tones were set by measuring levels and clipping. They'll change
    as they get tuned by ear.

## Save a tone

**Save** in the header, or ++ctrl+s++, writes your edits over the tone that's
loaded, including all its [scenes](scenes.md).

To keep the tone you started from, open the Tone Explorer, type a name next to
**Name** at the bottom, and press **Save as new**. It never replaces another tone:
if the name is taken, a number is added, such as `Fern Crunch 2`. That's also the
way to start a variation, or to rename a tone and then delete the old one.

**Revert** appears next to **Save** while a saved tone has edits. It puts the tone
back as it was saved, and **Undo** brings your edits back if you change your mind.

Moving to another tone never loses unsaved edits. Press **Undo** to step back to them.

A name needs at least one letter or number. Two names that differ only in
punctuation or case, like `Lead #2` and `lead 2`, share one file, so **Save as new**
treats the second as taken.

## When a file is missing

A tone still loads when a capture or cabinet IR it uses isn't in your library, for
example from a pack someone shared without its files. The unit that needs the file
shows a **!** and its panel says which file is missing.

- A missing capture passes your signal through until you press **Locate file** and
  choose the `.nam`.
- A missing cabinet IR plays the built-in cabinet until you press **Load cab IR**.
- TONE3000 files download again through your own account, as before.

## Delete a tone

Load the tone, then press **Delete tone** under its signal chain in the Tone Explorer,
and confirm. It stays on the rig until you load something else, so **Save** brings it
back. Factory tones you delete stay deleted.
