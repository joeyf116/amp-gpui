# Practice and tune

## Practice view

The **Practice** tab shows guitar tab split into sections, and each section can
link a saved tone. One click then loads the right sound for that part of the
song. A new library starts with a daily warm-up sheet.

### Import tab

Press **Import tab** and choose a plain text file. A line like `[Verse]` starts a
new section, and anything before the first one goes in a section called Tab.

```text
[Verse]
e|-------0---0---|
B|-----1---1---1-|
G|---2-----------|
D|-2-------------|
A|---------------|
E|---------------|

[Chorus]
e|---------------|
B|---------------|
G|-----------7---|
D|---7-7-9-9---9-|
A|-7-------------|
E|---------------|
```

Spacing is kept exactly, so your columns stay lined up.

### Link a tone to a section

1. Load the tone you want, and save it if it isn't saved. A link has to point to
   a saved tone.
2. In the section, press **Link current tone**.

The section now has a **Load** button with the tone's name on it. Links go by
name, so if you rename a tone, link it again.

## Tuner

Press **Tuner** or ++t++, and play one open string. Monitoring has to be running.
The tuner shows the nearest note, its frequency, and how many cents off you are.
It turns green within 3 cents.

The tuner reads your guitar before the **Input** knob, so it still works with the
input turned down. It can read down to 55 Hz, which covers a low E tuned down a
fifth.
