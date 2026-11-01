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

## Updates

The app's footer offers a newer release (`crates/amp-app/src/rig/update.rs`). It reads
`releases/latest` from the GitHub API, so prereleases are skipped, and it needs the
release to have `amp-gpui-setup.exe` with GitHub's asset digest. It runs the installer
with `/relaunch=1`, which `amp-gpui.iss` uses to reopen the app after a silent install.
The version in `Cargo.toml` is what it compares against, so a release has to bump it.

## Code signing

Tag builds sign the installer through the
[SignPath Foundation](https://signpath.org/) free program once
`SIGNPATH_ORGANIZATION_ID` is set. Without it, builds carry on unsigned. Only the
installer is signed, so each release needs one approval, not two. The certificate
names SignPath Foundation as the publisher, not this project.

Setting it up, once the application is accepted:

1. In SignPath, create project `amp-gpui`, connected to this repository with the
   GitHub Actions trusted build system, a signing policy `release-signing`, and an
   artifact configuration `installer`:

    ```xml
    <?xml version="1.0" encoding="utf-8"?>
    <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
      <zip-file>
        <pe-file path="amp-gpui-setup.exe">
          <authenticode-sign/>
        </pe-file>
      </zip-file>
    </artifact-configuration>
    ```

2. Create a CI user with submitter rights on that policy, and add its API token as the
   repository secret `SIGNPATH_API_TOKEN`.
3. Add the organization ID as the repository variable `SIGNPATH_ORGANIZATION_ID`.

The next tag then stops at the signing step until someone approves the request on
SignPath. It waits for up to an hour.

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
