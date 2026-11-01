# Generate a tone

Describe a sound and Claude builds a full signal chain for it: gate, drive, amp,
cabinet, and effects, with every control set. You download it as a tone pack and
import it into Amp GPUI.

Inside the app, the [tone assistant](assistant.md) does this and more. This page is
for trying it without an API key in the app.

<div id="tone-generator" markdown="0">
  <p>The tone generator needs JavaScript.</p>
</div>

<script src="../assets/generator/catalog.js"></script>
<script src="../assets/generator/core.js"></script>
<script src="../assets/generator/ui.js"></script>

## Add it to the app

1. Unzip the tone pack.
2. In Amp GPUI, open the Tone Explorer and choose **Library**, then **Import tones**.
3. Choose the unzipped folder. The tone appears in the Tone Explorer.

!!! warning "Start with your volume low"

    Nobody has heard a generated tone before you do. Levels are aimed at the
    factory tones, but they can come out a few decibels louder or quieter. Claude
    picks the settings from a description and can't listen to the result, so treat
    each tone as a starting point and adjust it by ear.

## What it can make

Generated tones use only the built-in blocks: the Understory amp, the built-in
cabinet voice, and the gate, compressor, drive, chorus, delay, and reverb. That means
they play right away with no files to download, and they're free to share.

They don't use `.nam` captures or impulse responses. If you ask for a real amp, you
get the Understory amp set to get as close as it can, and the tone's notes name the
gear it's reaching for. To go further, [swap in a capture](board.md) of that amp.

Wah, phaser, tremolo, pitch shifting, and stereo aren't available as blocks, so the
notes will say so if a sound depends on one.

## Ways to use Claude

There's no "Sign in with Claude" button on this site because
[Anthropic doesn't allow other sites to offer one](https://code.claude.com/docs/en/legal-and-compliance).
Each route on this page uses Claude in a way that's allowed:

| Route | You need | What it costs |
| --- | --- | --- |
| Sign in with your Claude account | A Claude account | Counts toward your plan's usage |
| Copy a prompt into Claude | A Claude account | Counts toward your plan's usage |
| Use your own API key | A key from the [Claude Console](https://platform.claude.com/) | Billed by Anthropic per request |

The first route opens the generator as a Claude artifact, which runs inside
claude.ai, so you're signing in to Claude itself. Neither this site nor Amp GPUI ever
sees your login.

!!! note "About API keys"

    The key route runs entirely in your browser. The page sends your key straight to
    `api.anthropic.com`, and doesn't save it or send it anywhere else. Still, a key
    typed into a web page is only as safe as that page, so use one you can revoke.

Your description goes to Anthropic on every route, and this site doesn't keep a
copy. Anthropic's terms for your account or key apply.

## Credits

The idea of describing a tone and getting a full chain comes from Thomas Lennon's
[Tone3000-PresetBuilder](https://github.com/tlennon-ie/Tone3000-PresetBuilder), a
Claude skill for the TONE3000 plugin. This generator is a separate design, built for
Amp GPUI's own preset format.
