# Tune and practice

Press **Practice tools** at the bottom right, or ++t++, to open the tuner,
metronome, and fretboard together.

![Practice tools, with the tuner, metronome, and fretboard](../assets/screens/tools.webp)

Monitoring has to be running for the tuner, the click, and the fretboard's live
notes. Press ++m++ first.

## Tuner

Play one open string. The needle shows how far off the note is, and the words under
it say which way to turn the peg. The needle centers and turns green within 3 cents
of the note. The six circles light up when you land on a standard tuning string.

The tuner reads your guitar before the **Input** knob, so it still works with the
input turned down. **Mute while tuning** silences your rig while the panel is open.

## Metronome

- **Start and stop:** press **Start**, or press ++k++ from anywhere in the app. While
  it's running, the footer shows the tempo and a light on the first beat.
- **Set the tempo:** press **+** and **−**, scroll over the number, or press **Tap
  tempo** in time. Tempo runs from 30 to 300 BPM.
- **Set the bar:** the first beat of each bar has a higher click. Choose from 1 to 12
  beats per bar.
- **Set the level:** the **Level** knob sets the click on its own, so **Output** can
  change without changing it.

The click runs on the audio thread, so it stays in time however busy the app is. It
goes straight to your speakers and doesn't pass through your rig.

## Fretboard

Choose a key and a scale, and the fretboard marks every note of that scale on the
first 15 frets. The root is green. **Showing notes** switches the labels between note
names and intervals, such as `b3` and `5`, and the row at the right spells the scale
out.

Play a note and it lights up wherever that exact pitch can be fretted, then fades.

| Color | Meaning |
| --- | --- |
| Green | The root of the key |
| Gray | Another note of the scale |
| White | What you just played, in the scale |
| Amber | What you just played, outside the scale |

The note comes from the tuner, so the fretboard follows one note at a time. Chords
show unreliably, and very fast runs can skip notes.

The included scales are minor and major pentatonic, blues, major, natural, harmonic,
and melodic minor, and the Dorian, Phrygian, Lydian, Mixolydian, and Locrian modes.
