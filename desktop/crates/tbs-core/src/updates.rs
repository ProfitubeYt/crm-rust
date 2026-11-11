//! Finding, fetching and verifying a newer build of the client.
//!
//! Releases live in the same public repository the Electron client updates
//! from. Electron builds there stay on 1.x, and the native client starts at
//! 2.0, so an old client never picks up a native installer by accident.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const LATEST: &str = "https://api.github.com/repos/asintiko/parcer20-updates/releases/latest";
const INSTALLER_PREFIX: &str = "TBSparcer-Native-Setup-";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub file_name: String,
    pub size: u64,
    /// `sha256:<hex>`, when the host publishes one.
    pub digest: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<GithubAsset>,
}

fn parts(version: &str) -> Vec<u64> {
    version.trim_start_matches('v').split(['.', '-', '+']).map_while(|part| part.parse().ok()).collect()
}

/// Compared number by number: `1.10.0` is newer than `1.9.0`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parts(candidate), parts(current));
    let width = a.len().max(b.len());
    a.resize(width, 0);
    b.resize(width, 0);
    a > b
}

fn pick(release: GithubRelease) -> Option<Release> {
    let asset = release.assets.into_iter().find(|a| a.name.starts_with(INSTALLER_PREFIX) && a.name.ends_with(".exe"))?;
    Some(Release {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        notes: release.body.unwrap_or_default(),
        url: asset.browser_download_url,
        file_name: asset.name,
        size: asset.size,
        digest: asset.digest,
    })
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("TBSparcer-Desktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())
}

/// `None` when the newest published build is the one running, or older.
pub async fn check(current: &str) -> Result<Option<Release>, String> {
    let reply = client()?
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "Сервер обновлений недоступен".to_owned())?;
    if reply.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !reply.status().is_success() {
        return Err(format!("Сервер обновлений ответил {}", reply.status().as_u16()));
    }
    let latest: GithubRelease = reply.json().await.map_err(|error| error.to_string())?;
    Ok(pick(latest).filter(|release| is_newer(&release.version, current)))
}

fn matches_digest(expected: Option<&str>, actual_hex: &str) -> bool {
    match expected.and_then(|value| value.strip_prefix("sha256:")) {
        Some(hex) => hex.eq_ignore_ascii_case(actual_hex),
        None => true,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Writes next to its final name first, so a broken download is never run.
pub async fn download(release: &Release, dir: &Path, mut on_progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|error| error.to_string())?;
    let target = dir.join(&release.file_name);
    let partial = target.with_extension("part");
    let mut reply = client()?.get(&release.url).send().await.map_err(|error| error.to_string())?;
    if !reply.status().is_success() {
        return Err(format!("Сервер обновлений ответил {}", reply.status().as_u16()));
    }
    let total = reply.content_length().unwrap_or(release.size);
    let mut file = tokio::fs::File::create(&partial).await.map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = reply.chunk().await.map_err(|error| error.to_string())? {
        file.write_all(&chunk).await.map_err(|error| error.to_string())?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        on_progress(received, total);
    }
    file.flush().await.map_err(|error| error.to_string())?;
    drop(file);
    if !matches_digest(release.digest.as_deref(), &hex(&hasher.finalize())) {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err("Файл обновления повреждён, контрольная сумма не сошлась".to_owned());
    }
    tokio::fs::rename(&partial, &target).await.map_err(|error| error.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("v2.10.0", "2.9.9"));
        assert!(is_newer("2.1", "2.0.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.6.2", "2.0.0"));
    }

    #[test]
    fn only_the_native_installer_counts() {
        let electron = GithubAsset { name: "TBSparcer-Setup-1.6.2.exe".into(), ..Default::default() };
        let release = GithubRelease { tag_name: "v1.6.2".into(), assets: vec![electron], ..Default::default() };
        assert!(pick(release).is_none());

        let native = GithubAsset {
            name: "TBSparcer-Native-Setup-2.1.0.exe".into(),
            browser_download_url: "https://example.test/a".into(),
            size: 5,
            digest: Some("sha256:ab".into()),
        };
        let release = GithubRelease { tag_name: "v2.1.0".into(), assets: vec![native], ..Default::default() };
        let found = pick(release).unwrap();
        assert_eq!(found.version, "2.1.0");
        assert_eq!(found.size, 5);
    }

    #[test]
    fn a_digest_must_match_when_there_is_one() {
        assert!(matches_digest(None, "abc"));
        assert!(matches_digest(Some("sha256:ABC"), "abc"));
        assert!(!matches_digest(Some("sha256:def"), "abc"));
    }

    #[test]
    fn hashes_are_lowercase_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
