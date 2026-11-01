# Connect your interface

!!! warning "Start quiet"

    Turn your speakers down before the first start. The output is limited so it
    can't clip, but a limited signal can still be loud.

## Plug in

1. Plug your guitar into an instrument input on your interface. If the input has
   a Hi-Z or instrument switch, turn it on.
2. Plug your speakers or headphones into the interface.
3. **Turn off direct monitoring** on the interface. Otherwise you hear your dry
   guitar underneath the amp.
4. Set the interface's input gain so that hard picking stays out of the red.

## Choose your devices

The bar along the top of the window holds the audio settings.

=== "With an ASIO driver"

    This is the best choice if your interface has one, and it needs speakers on
    the interface.

    1. Install your interface maker's ASIO driver.
    2. Under **Audio driver**, choose it. The list fills in a moment after the
       app opens.
    3. Under **Channel**, choose the input your guitar is plugged into.

    **Guitar in** and **Speakers out** disappear, because ASIO runs one
    interface for both.

=== "With Windows audio"

    Use this if you have no ASIO driver, or if you want to play through speakers
    that aren't on the interface, such as a monitor or a laptop.

    1. Leave **Audio driver** on **Windows audio**.
    2. Under **Guitar in**, choose your interface. Some interfaces list each
       channel as its own device, such as "Mic 1".
    3. Under **Channel**, choose the input your guitar is plugged into.
    4. Under **Speakers out**, choose your speakers.

## Play

Press **Start monitoring**, or press ++m++. The app opens on a tone called Moss
Clean, so you should hear a warm clean sound.

Check the **In** and **Out** meters at the top right while you play:

- If **In** barely moves, you probably picked the wrong channel, or the
  interface gain is too low.
- If a meter turns red, the level is close to full scale. Turn down the **Input**
  or **Output** knob beside the meters.

The app remembers your devices and levels for next time.

Next, [get your latency down](monitoring.md).
