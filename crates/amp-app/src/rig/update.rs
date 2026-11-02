//! Offers new releases in the footer. It asks GitHub what the latest release is
//! once at startup, and only downloads when the player presses the button. The
//! download has to match the digest GitHub reports before it runs.

use super::Rig;
use gpui_kit::{
    component::{Disableable, Sizable, button::Button},
    *,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use ureq::Agent;

const LATEST: &str = "https://api.github.com/repos/joeyf116/amp-gpui/releases/latest";
const DOWNLOADS: &str = "https://github.com/joeyf116/amp-gpui/releases/download/";
const INSTALLER: &str = "amp-gpui-setup.exe";
const USER_AGENT: &str = concat!("amp-gpui/", env!("CARGO_PKG_VERSION"));
/// Waits this long after opening, so the check doesn't compete with startup.
const CHECK_DELAY: Duration = Duration::from_secs(5);
/// Runs without a window of questions, then opens the new version.
const INSTALL_ARGUMENTS: [&str; 4] = ["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/relaunch=1"];

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

/// A release newer than this app.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Release {
    version: String,
    url: String,
    /// In lowercase hex.
    sha256: String,
}

/// What the footer offers.
#[derive(Default)]
pub(super) enum Update {
    #[default]
    None,
    Available(Release),
    /// Waiting for a second press, as updating closes the app with unsaved edits.
    Confirming(Release),
    Downloading {
        /// The share done so far, as the bits of a float.
        progress: Arc<AtomicU32>,
    },
    Failed(SharedString),
}

/// Reads a version such as `v1.2.3` as numbers. A prerelease counts as not a version.
fn version_numbers(text: &str) -> Option<Vec<u32>> {
    text.trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (version_numbers(candidate), version_numbers(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// Picks out the installer, if `release` is newer than `current` and has one
/// that came from this project and can be checked.
fn newer_release(release: GithubRelease, current: &str) -> Option<Release> {
    if !is_newer(&release.tag_name, current) {
        return None;
    }
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == INSTALLER)?;
    let sha256 = asset.digest?.strip_prefix("sha256:")?.to_lowercase();
    asset
        .browser_download_url
        .starts_with(DOWNLOADS)
        .then(|| Release {
            version: release.tag_name.trim_start_matches('v').to_owned(),
            url: asset.browser_download_url,
            sha256,
        })
}

fn agent(body_timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(body_timeout))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

fn check() -> Option<Release> {
    let mut response = agent(Duration::from_secs(30)).get(LATEST).call().ok()?;
    let release: GithubRelease = response.body_mut().read_json().ok()?;
    newer_release(release, env!("CARGO_PKG_VERSION"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Saves the installer to a temporary file, refusing anything that doesn't match.
fn download(release: &Release, progress: &AtomicU32) -> Result<PathBuf, String> {
    let describe = |error: &dyn std::fmt::Display| format!("Update failed: {error}");
    let response = agent(Duration::from_secs(600))
        .get(&release.url)
        .call()
        .map_err(|error| describe(&error))?;
    let length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let mut reader = response.into_body().into_reader();
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 256 * 1024];
    loop {
        let count = reader.read(&mut chunk).map_err(|error| describe(&error))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(length) = length {
            let share = bytes.len() as f32 / length.max(1) as f32;
            progress.store(share.to_bits(), Ordering::Relaxed);
        }
    }
    if hex(&Sha256::digest(&bytes)) != release.sha256 {
        return Err("Update failed: the download didn't match its checksum".to_owned());
    }
    let path = std::env::temp_dir().join(format!("amp-gpui-setup-{}.exe", release.version));
    fs::write(&path, bytes).map_err(|error| describe(&error))?;
    Ok(path)
}

impl Rig {
    /// Looks for a new release in the background. Being offline isn't worth
    /// mentioning, so a failed check stays quiet. A build from source has no
    /// installer to replace it, so only release builds look.
    pub(super) fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if cfg!(debug_assertions) || !self.settings.check_updates {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DELAY).await;
            let release = cx.background_executor().spawn(async { check() }).await;
            if let Some(release) = release {
                let _ = this.update(cx, |this, cx| {
                    this.update = Update::Available(release);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn press_update(&mut self, cx: &mut Context<Self>) {
        let unsaved = self.edited();
        match std::mem::take(&mut self.update) {
            Update::Available(release) if unsaved => self.update = Update::Confirming(release),
            Update::Available(release) | Update::Confirming(release) => {
                self.install_update(release, cx);
            }
            other => self.update = other,
        }
        cx.notify();
    }

    fn install_update(&mut self, release: Release, cx: &mut Context<Self>) {
        let progress = Arc::new(AtomicU32::new(0));
        self.update = Update::Downloading {
            progress: progress.clone(),
        };
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { download(&release, &progress) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(installer) => {
                    this.save_settings(cx);
                    let started = std::process::Command::new(installer)
                        .args(INSTALL_ARGUMENTS)
                        .spawn();
                    match started {
                        Ok(_) => cx.quit(),
                        Err(error) => {
                            this.update = Update::Failed(format!("Update failed: {error}").into());
                            cx.notify();
                        }
                    }
                }
                Err(error) => {
                    this.update = Update::Failed(error.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The footer's offer of a new version, if there is one.
    pub(super) fn render_update(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label: SharedString = match &self.update {
            Update::None => return None,
            Update::Failed(error) => error.clone(),
            Update::Available(release) => format!("Update to {}", release.version).into(),
            Update::Confirming(_) => "Update and lose unsaved edits".into(),
            Update::Downloading { progress } => {
                let share = f32::from_bits(progress.load(Ordering::Relaxed));
                format!("Downloading update, {:.0}%", share * 100.0).into()
            }
        };
        let busy = matches!(self.update, Update::Downloading { .. } | Update::Failed(_));
        Some(
            Button::new("update")
                .small()
                .outline()
                .label(label)
                .tooltip("Installs the new version and reopens the app")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.press_update(cx)))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{GithubAsset, GithubRelease, Release, hex, is_newer, newer_release};
    use sha2::{Digest, Sha256};

    fn release(tag: &str, name: &str, url: &str, digest: Option<&str>) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_owned(),
            assets: vec![GithubAsset {
                name: name.to_owned(),
                browser_download_url: url.to_owned(),
                digest: digest.map(str::to_owned),
            }],
        }
    }

    const GOOD_URL: &str =
        "https://github.com/joeyf116/amp-gpui/releases/download/v0.2.0/amp-gpui-setup.exe";

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("v0.3.0-beta.1", "0.2.0"));
    }

    #[test]
    fn a_new_release_with_a_checkable_installer_is_offered() {
        let found = newer_release(
            release(
                "v0.2.0",
                "amp-gpui-setup.exe",
                GOOD_URL,
                Some("sha256:AB12"),
            ),
            "0.1.0",
        );
        assert_eq!(
            found,
            Some(Release {
                version: "0.2.0".to_owned(),
                url: GOOD_URL.to_owned(),
                sha256: "ab12".to_owned(),
            })
        );
    }

    #[test]
    fn anything_that_cant_be_checked_is_not_offered() {
        let offer = |asset: GithubRelease| newer_release(asset, "0.1.0");
        let name = "amp-gpui-setup.exe";
        assert_eq!(offer(release("v0.2.0", name, GOOD_URL, None)), None);
        assert_eq!(
            offer(release("v0.2.0", "other.zip", GOOD_URL, Some("sha256:ab"))),
            None
        );
        let elsewhere = "https://example.com/amp-gpui-setup.exe";
        assert_eq!(
            offer(release("v0.2.0", name, elsewhere, Some("sha256:ab"))),
            None
        );
        assert_eq!(
            offer(release("v0.1.0", name, GOOD_URL, Some("sha256:ab"))),
            None
        );
    }

    #[test]
    fn digests_compare_in_the_form_github_writes() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
