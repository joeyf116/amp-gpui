# Troubleshooting

## I hear a clean guitar under the amp

Your interface's direct monitoring is on, so both the dry and processed signals
reach your speakers. Turn it off on the interface or in its mixer app.

## I don't hear anything

1. Check that **Start monitoring** has turned into **Stop monitoring**.
2. Play and watch the **Input** meter. If it doesn't move, check **Channel**, the
   interface's input gain, and the guitar's volume knob.
3. If **Input** moves but **Output** doesn't, check that a gate isn't closed with its
   threshold set too high, and that the **Output** knob isn't turned down.
4. If the **Output** meter moves but the speakers are silent, check **Speakers
   out** in **Audio setup**, or the
   interface's output and headphone levels.

On **Low latency**, other apps go silent on those devices. That's expected, and
it stops when you stop monitoring.

## Other apps went silent

**Low latency** takes the devices over. Stop monitoring to get them back, or press
the button in **Audio setup** to switch to **Shared audio**.

## It says "Using a slower route"

The route you chose couldn't start, so the app fell back to a slower one. The
usual cause is another app holding the device, so close it and press ++m++ twice.

## My latency went up over time

If the computer stalls, the app grows the queue between the devices to prevent
more dropouts, and that stays until you restart monitoring. Press ++m++ twice.

## The glitch count keeps climbing

- Raise your ASIO driver's buffer a step.
- Lower **Quality** on A2 captures.
- Use fewer captures, since they're the heaviest units on the board.
- Run a [release build](install.md#build-and-run).

## ASIO isn't in the list

The driver list fills in a moment after the app opens. If yours still isn't there,
reinstall your interface's ASIO driver, and check that the app wasn't built with
`--no-default-features`.

## ASIO won't start

Only one app at a time can usually use an ASIO driver. Close your DAW or any other
audio app, then try again.

## The app says a device uses unsupported samples

**Shared audio** only supports 32-bit float. Try **Low latency** or ASIO, which
handle more formats.

## "this model was captured at ... Hz"

The capture isn't 48 kHz, and it needs to be. See
[Add your own amps and pedals](board.md#add-your-own-amps-and-pedals).

## A tone says a file is missing from the library

The tone refers to a capture or impulse response that isn't in this library. Ask
for a [tone pack](sharing.md#share-one-tone-tone-packs), which includes the files.

## The assistant says the API key wasn't accepted

The key was mistyped or revoked. In the assistant's **Settings**, press **Remove key**
and paste a new one from the [Claude Console](https://platform.claude.com/settings/keys).
If it says the key is out of credit, add credit there.

## The assistant can't find a capture I know is on TONE3000

It can't search their catalog. Press **Browse TONE3000** when it offers, or ask it to
open the picker, and choose the tone yourself. It'll carry on from your pick.

## It says it's using a temporary library

The library folder you chose can't be opened, usually because its drive isn't there.
Reconnect it, or choose another with **Library** in the Tone Explorer.

## The app crashed

Details go to `crash.log` in `%APPDATA%\amp-gpui\data`, beside your settings. Attach
that file if you report it.

## It sounds like there are two cabinets

Your capture already includes one. Remove the **Cabinet** from the board.

## A pack won't import

If the error says a file doesn't match the hash in its preset, the file changed
after export, so ask for a fresh pack. The app won't import part of a pack.

## A tone "needs a newer version of this app"

The tone was saved by a newer version. Pull the latest code and rebuild.

<small>![ASIO compatible](../assets/asio-compatible.png){ .asio-logo width="80" }<br>ASIO is a registered trademark of Steinberg Media Technologies GmbH.</small>
