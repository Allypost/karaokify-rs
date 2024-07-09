use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client as ReqwestClient;
use serde::Deserialize;
use tracing::{debug, info, trace};
use url::Url;

use super::Handler;
use crate::helpers::{domain::DomainParser, download::download_file_inferred};

const URL_BASE: &str = "https://spotifydown.com";
const API_BASE: &str = "https://api.spotifydown.com";
static PATH_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"/track/(?<id>[a-zA-Z0-9]+)").expect("Invalid regex"));

#[derive(Debug)]
pub struct SpotifydownProvider;

#[async_trait::async_trait]
impl Handler for SpotifydownProvider {
    #[tracing::instrument(skip(self, song_url), fields(url = ?song_url.as_str()))]
    async fn download(&self, download_dir: &Path, song_url: &Url) -> anyhow::Result<PathBuf> {
        debug!("Downloading song");

        let download_url = tryhard::retry_fn(|| Self::get_download_url(song_url))
            .retries(5)
            .fixed_backoff(Duration::from_secs(2))
            .on_retry(|_attempt, _next_delay, err| {
                let e = err.to_string();

                async move {
                    debug!(?e, "Retrying song download");
                }
            })
            .await
            .map_err(|e| {
                if let Some(e) = e.downcast_ref::<reqwest::Error>() {
                    if e.is_timeout() {
                        return anyhow::anyhow!(
                            "Timeout downloading song. Download provider may be down."
                        );
                    }
                }
                info!(?e, "Failed to download song");
                anyhow::anyhow!("Failed to download song from provider")
            })?;

        debug!(?download_url, "Download URL found. Downloading song.");

        Self::download_file(download_dir, &download_url).await
    }

    async fn supports(&self, song_url: &Url) -> bool {
        let Some(root) = DomainParser::get_domain_root(song_url) else {
            return false;
        };

        root == "spotify.com"
    }
}

impl SpotifydownProvider {
    pub async fn get_download_url(song_url: &Url) -> anyhow::Result<String> {
        let Some(track_id) = PATH_REGEX
            .captures(song_url.path())
            .and_then(|x| x.name("id"))
        else {
            anyhow::bail!("Invalid Spotify URL");
        };

        trace!(?track_id, "Got track ID from song URL");

        let api_url = format!("{API_BASE}/download/{id}", id = track_id.as_str());
        trace!(?api_url, "Got API URL for song download request");
        let res = ReqwestClient::new()
            .get(api_url)
            .timeout(Duration::from_secs(5))
            .header("origin", URL_BASE)
            .header("referer", URL_BASE)
            .send()
            .await?
            .json::<DownloadResponse>()
            .await?;

        trace!(?res, "Got download response");

        match res {
            DownloadResponse::Error { message } => {
                Err(anyhow::anyhow!(message).context("Failed to get song download link"))
            }

            DownloadResponse::Success { link } => Ok(link),
        }
    }

    pub async fn download_file(download_dir: &Path, url: &str) -> anyhow::Result<PathBuf> {
        let download_path = download_dir.join("some song.mp3");

        trace!(?download_path, "Downloading song");
        let download_path = download_file_inferred(&download_path, url).await?;
        trace!(?download_path, "Downloaded song");

        Ok(download_path)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DownloadResponse {
    Success { link: String },
    Error { message: String },
}
