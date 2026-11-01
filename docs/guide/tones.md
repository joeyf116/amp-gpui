# Load and save tones

A tone is a saved board: every unit on it, in order, with its settings. The
**Tones** list is on the left side of the window.

## Load a tone

Click a tone in the list, or press ++page-up++ and ++page-down++ to step through
them. Tones load in the background and crossfade in, so you can switch while
you're playing. Delay and reverb tails from the old tone stop as it fades out.

If a tone can't load, for example because a file it needs is missing, the current
tone keeps playing and an error line tells you why.

The app reopens the last tone you used.

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

1. Type a name in the box above the tone list.
2. Press the button under it, which reads **Save tone**, or **Save changes** when
   the board has edits.

Saving under the name of an existing tone replaces it. Saving under a new name
makes a copy, which is the way to rename a tone or start a variation.

A name needs at least one letter or number. Two names that differ only in
punctuation or case, like `Lead #2` and `lead 2`, share one file, so the second
save replaces the first.

## Delete a tone

There's no delete button yet. Delete the tone's `.json` file from the library's
`presets` folder, then restart the app. Factory tones you delete stay deleted.
