# Releases

Installers are built by `.github/workflows/release.yml` with
[Inno Setup](https://jrsoftware.org/isinfo.php), from `amp-gpui.iss`.

## Cut a release

1. Set `version` under `[workspace.package]` in `Cargo.toml`, and commit it to
   `main`.
2. Tag that commit and push the tag. The tag is the version with a `v` in front.

    ```bash
    git tag v0.2.0
    ```

    ```bash
    git push origin v0.2.0
    ```

The workflow tests and builds, then publishes a GitHub Release with
`amp-gpui-setup.exe` and its checksum. The docs
[Download](https://joeyf116.github.io/amp-gpui/download/) page reads the release
list from GitHub, so it needs no change. A version with a suffix, such as
`0.2.0-beta.1`, is marked as a prerelease and doesn't become the main download.

To rehearse without publishing, push a branch named `release/<anything>` or run
the workflow by hand. The installer lands on the run page as an artifact.

## ASIO

Published installers leave ASIO out, through `ASIO_ARGS` at the top of the
workflow. Steinberg's ASIO SDK allows distribution of a built app in one of two
ways:

- under GPLv3, which means the whole app is released under GPLv3, or
- under the proprietary license, which needs an agreement signed by Steinberg
  before publishing.

See `LICENSE.txt` in the SDK. Once either is in place, empty `ASIO_ARGS`, and
take the "Audio drivers in the installer" section off `docs/download.md`. An ASIO
build also needs the attribution from the SDK's usage guidelines: "ASIO is a
registered trademark of Steinberg Media Technologies GmbH."

## Code signing

Installers aren't signed yet, so Windows SmartScreen warns about them.
