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
`amp-gpui-setup.exe`, its checksum, and the ASIO SDK source. The docs
[Download](https://joeyf116.github.io/amp-gpui/download/) page reads the release
list from GitHub, so it needs no change. A version with a suffix, such as
`0.2.0-beta.1`, is marked as a prerelease and doesn't become the main download.

To rehearse without publishing, run the workflow by hand, or push a branch named
`release/<anything>` that changes `Cargo.lock` or the packaging files. The
installer lands on the run page as an artifact.

Every build also runs `test-installer.ps1`, which installs, upgrades from an older
version, starts the app, and uninstalls.

## ASIO

Installers include ASIO, using Steinberg's ASIO SDK under its GPLv3 option. That's
why the app is GPLv3, and it comes with two duties:

- **Source.** GPLv3 covers the SDK code in the binary, and the repo doesn't hold
  that code. So the workflow publishes the SDK's `common` and `host` folders, the
  ones the build compiles, as `asio-sdk-source.zip` on each release.
- **Trademark.** The ASIO name and logo follow the usage guidelines in the SDK.
  Each docs page that mentions ASIO ends with the logo and this line, and the
  Audio setup panel shows the line:
  "ASIO is a registered trademark of Steinberg Media Technologies GmbH."

## Code signing

Installers aren't signed yet, so Windows SmartScreen warns about them.
