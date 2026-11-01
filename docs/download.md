---
title: Download
---

# Download

Amp GPUI runs on 64-bit Windows 10 and 11. The installer doesn't need admin
rights, and installing a new version over an old one keeps your settings and
tones.

<div id="releases" data-repo="joeyf116/amp-gpui" markdown>

[Download the latest version](https://github.com/joeyf116/amp-gpui/releases/latest/download/amp-gpui-setup.exe){ .md-button .md-button--primary }

[All releases on GitHub](https://github.com/joeyf116/amp-gpui/releases)

</div>

<script src="assets/download/releases.js"></script>

## Windows security warning

The installer isn't code signed yet, so Windows SmartScreen may stop it with
"Windows protected your PC". Choose **More info**, then **Run anyway**.

Each release also has an `amp-gpui-setup.exe.sha256` file. To check your
download against it, run this from the folder you saved it in, then compare the
result with the contents of that file:

```bash
certutil -hashfile amp-gpui-setup.exe SHA256
```

## License

Amp GPUI is free software under the
[GNU General Public License, version 3](https://github.com/joeyf116/amp-gpui/blob/main/LICENSE).
Each release on GitHub has the source for that version, including the ASIO SDK
source it was built from.

Next, [connect your interface](guide/setup.md).

<small>![ASIO compatible](assets/asio-compatible.png){ .asio-logo width="80" }<br>ASIO is a registered trademark of Steinberg Media Technologies GmbH.</small>
