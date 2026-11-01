# Share tones

Your tone library is an ordinary folder. There's no account and no server, so you
can share it any way you share files. There are two ways to do it.

## Share one tone: tone packs

A tone pack is a folder with one tone and the files it needs. Files from
[TONE3000](tone3000.md#sharing-tones-that-use-them) stay out of it.

**To export,** load the tone, press **Export** under the tone list, and choose an
empty folder. Zip that folder and send it however you like.

**To import,** unzip the pack, press **Import**, and choose its folder. Every file
is re-hashed before anything is added, so a pack can't swap a file for something
else. If any file fails, nothing is imported.

Imported tones play straight away. Their captures don't appear in your gear
drawer, though, because it only lists captures you've added yourself. Press **Add
.nam** to add them.

The [tone generator](generate.md) produces this same kind of pack.

## Share everything: the library folder

Press **Folder** and choose where the library should live. Any folder works,
including:

- a git repository
- a folder on a synced drive
- a network share

Everyone who points the app at that folder sees the same tones, captures, and
practice sheets. If the folder has no tones in it, the factory tones are added
the next time the app starts.

Because each file is named by a hash of its contents, a tone means exactly the
same thing on every machine. See [Presets and the library](../internals/presets.md)
for the details.

!!! warning "Check licenses before you share"

    Captures and impulse responses often come with their own license. Tones made
    only from built-in blocks carry no files, so they're always fine to share.
