# Monitoring and latency

Latency is the gap between picking a note and hearing it. It's the reason this
app exists in its current shape: past about 10 to 15 ms, a guitar stops feeling
connected to your hands.

For the steps on their own, see [Low latency setup](low-latency.md).

## The three routes

| Route | Reported latency | Other apps' sound | Needs |
| --- | --- | --- | --- |
| **ASIO** | About 8 ms | Depends on the driver | An ASIO driver, with input and output on the same interface |
| **Low latency** (WASAPI exclusive) | About 17 ms | Silent on those devices | Any Windows audio devices |
| **Shared audio** (WASAPI shared) | About 40 ms or more | Keeps playing | Any Windows audio devices |

These figures were reported by the drivers on a Behringer FLOW 8, with ASIO at a
64-frame buffer. Your interface will differ.

Choose a route like this:

- Have an ASIO driver and speakers on the interface? Use ASIO.
- Playing through speakers somewhere else? Use **Low latency**.
- Need to hear a backing track from another app on the same device? Use **Shared
  audio**, or choose ASIO if your driver lets other apps play.

The **Low latency** button is in **Audio setup**, at the bottom left of the
window. Press it to switch to **Shared audio** and back. It's hidden while an ASIO driver is selected.

## If a route is busy

The app starts on the fastest route you've chosen and falls back to the next one
if it can't open. Something else may be holding a device, and playing with more
latency beats not playing. When that happens, an error line tells you which
route failed and says "Using a slower route."

## Reading the display

While monitoring, this line appears under the meters:

```text
8 ms latency (ASIO), 0 glitches
```

- **Latency** adds up the delay the drivers report and the queue between the two
  devices.
- **The route name** in brackets shows which route actually started.
- **Glitches** counts dropouts and recoveries since you started. A few at startup
  are nothing to worry about. A count that keeps climbing means the computer
  isn't keeping up.

!!! note "The real figure can be higher"

    The app only counts what drivers report. Anything after that isn't included,
    such as the delay inside a monitor's HDMI speakers. Bluetooth is the worst
    case, and it's far too slow to play through.

## Lowering latency

1. Use ASIO if you can.
2. Set your driver's buffer as low as it will go without glitches. ASIO drivers
   usually have their own control panel. The app asks for 64 frames.
3. Close anything heavy that's running in the background.
4. Use a [release build](install.md#build-and-run). In a debug build, part of the
   audio path runs unoptimized.
5. On a capture that has one, lower the amp's **Quality** knob. See
   [NAM A2 captures](board.md#nam-a2-captures).

If the queue runs dry, the app makes it slightly bigger to stop the glitching.
That extra delay lasts until you restart monitoring, so press ++m++ twice after
fixing whatever was causing dropouts.
