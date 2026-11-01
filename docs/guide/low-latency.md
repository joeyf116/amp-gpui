# Low latency setup

Follow these steps in order to get the lowest latency from Amp GPUI on Windows.
The goal is under 15 ms, and under 10 ms feels like playing through a real amp.

You need a guitar, a USB audio interface, and speakers or headphones plugged into
that interface.

## 1. Install the app

Get the installer from the [Download](../download.md) page.

!!! note "ASIO in the installer"

    Installers don't include ASIO yet. To use step 3 today,
    [build from source](install.md). Otherwise skip to step 4, which gets you to
    about 17 ms on the interface we tested.

## 2. Set up your hardware

1. Plug the interface straight into the computer, not through a USB hub.
2. Plug your speakers or headphones into the interface. Avoid Bluetooth, which
   is far too slow to play through. HDMI speakers add their own delay, which the
   app can't see.
3. Plug your guitar into an instrument input. If it has a Hi-Z or instrument
   switch, turn it on.
4. Turn off direct monitoring on the interface, or you'll hear your dry guitar
   underneath the amp.
5. Turn your speakers down before you start.

## 3. Use your interface's ASIO driver

This is the fastest route, at about 8 ms on the interface we tested.

1. Install the ASIO driver from your interface maker's website.
2. Open Amp GPUI and press **Audio setup** at the bottom left.
3. Under **Audio driver**, choose your interface's driver. The list fills in a
   moment after the app opens.
4. Under **Channel**, choose the input your guitar is plugged into.
5. Open your driver's own control panel and set its buffer as low as it goes
   without glitches. Start at 64 samples, and go up a step if you hear crackles.

Then skip to step 5.

## 4. No ASIO driver? Use Low latency

1. Press **Audio setup** at the bottom left.
2. Leave **Audio driver** on **Windows audio**.
3. Under **Guitar in**, choose your interface, and under **Channel**, your input.
4. Under **Speakers out**, choose your speakers.
5. Check the route button says **Low latency**. If it says **Shared audio**, press
   it.

Other apps go silent on those devices while you monitor.

If the route won't open, go to the **Sound** control panel in Windows and open
your device's **Properties**. On the **Advanced** tab, tick **Allow applications
to take exclusive control of this device**. Do that for both the input and the
output.

## 5. Prepare Windows

1. Plug in your laptop if you're using one.
2. In **Settings**, under **System** and then **Power**, set the power mode to
   **Best performance**.
3. Close anything heavy, such as browsers with lots of tabs, games, and video
   calls.

## 6. Check your result

Press **Start monitoring** at the top right, or press ++m++, and play. This line
appears under the meters:

```text
8 ms latency (ASIO), 0 glitches
```

- **Check the route in brackets.** If it isn't the one you set up, the app fell
  back to a slower one, and an error line says which route failed.
- **Compare the latency** with the table below.
- **Watch the glitch count.** A few at startup are fine. If it keeps climbing,
  raise your driver's buffer a step, then press ++m++ twice to restart monitoring.

| Route | Latency on a Behringer FLOW 8 |
| --- | --- |
| ASIO, 64 samples | About 8 ms |
| Low latency | About 17 ms |
| Shared audio | About 40 ms or more |

Your interface will differ, and the display only counts what the drivers report.

## Still too slow?

- On a capture that has one, lower the amp's **Quality** knob. See
  [NAM A2 captures](board.md#nam-a2-captures).
- See [Monitoring and latency](monitoring.md) for how the routes work, and
  [Troubleshooting](troubleshooting.md) for other problems.
