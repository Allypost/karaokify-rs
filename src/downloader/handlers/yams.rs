use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use tracing::{debug, trace, warn};
use url::Url;

use super::Handler;
use crate::helpers::download::download_file;

const API_URL: &str = "https://yams.tf/api";
const QUALITY_MAP: &[(&str, &str)] = &[
    ("spotify", "very_high"),
    ("qobuz", "27"),
    ("tidal", "3"),
    ("apple", "high"),
    ("deezer", "2"),
    ("youtube", "0"),
];

type YamsId = u64;

#[derive(Debug, Deserialize)]
struct YamsInitialResponse {
    id: YamsId,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct YamsStatusResponse {
    id: YamsId,
    status: String,
    progress: Option<serde_json::Value>,
    error: Option<String>,
    url: Option<String>,
}

#[derive(Debug)]
pub struct YamsProvider;

#[async_trait::async_trait]
impl Handler for YamsProvider {
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
                        warn!(
                            ?e,
                            "Timeout downloading song. Download provider may be down."
                        );
                        return anyhow::anyhow!(
                            "Timeout downloading song. Download provider may be down."
                        );
                    }
                }
                warn!(?e, "Failed to download song");
                anyhow::anyhow!("Failed to download song from provider")
            })?;
        debug!(?download_url, "Download URL found. Downloading song zip.");

        let song_zip_path = Self::download_song_zip(download_dir, &download_url).await?;
        debug!(
            ?song_zip_path,
            "Song zip downloaded. Extracting song from zip."
        );
        let song_file_path =
            Self::extract_song_from_zip(download_dir.to_path_buf(), song_zip_path.clone()).await?;

        debug!(?song_file_path, "Song downloaded and extracted");

        let _ = tokio::fs::remove_file(song_zip_path).await;

        Ok(song_file_path)
    }

    async fn supports(&self, song_url: &Url) -> bool {
        Self::get_quality(song_url).is_some()
    }
}

impl YamsProvider {
    #[tracing::instrument]
    async fn extract_song_from_zip(
        download_dir: PathBuf,
        zip_path: PathBuf,
    ) -> anyhow::Result<PathBuf> {
        trace!("Extracting song from zip");
        tokio::task::spawn_blocking(move || {
            let zip_file = std::fs::File::open(zip_path)?;
            let mut zip = zip::ZipArchive::new(zip_file)?;

            trace!("Finding file in zip");

            for i in 0..zip.len() {
                let mut file_in_zip = zip.by_index(i)?;

                if !file_in_zip.is_file() {
                    continue;
                }

                trace!(f = ?file_in_zip.name(), "Found file in zip");

                let file_name = match file_in_zip
                    .enclosed_name()
                    .and_then(|x| x.file_name().map(std::ffi::OsStr::to_os_string))
                {
                    Some(x) => x,
                    None => continue,
                };

                if file_name.to_string_lossy().starts_with('.') {
                    continue;
                }

                trace!(?file_name, "Extracing file from zip");

                let file_path = download_dir.join(file_name);
                let mut file_on_disk = std::fs::File::create(&file_path)?;

                std::io::copy(&mut file_in_zip, &mut file_on_disk)?;

                return Ok(file_path);
            }

            anyhow::bail!("Could not find file in zip");
        })
        .await?
    }

    #[tracing::instrument]
    async fn download_song_zip(download_dir: &Path, download_url: &str) -> anyhow::Result<PathBuf> {
        trace!("Downloading song zip");
        let download_path = download_dir.join("file.zip");

        download_file(&download_path, download_url).await?;

        Ok(download_path)
    }

    #[tracing::instrument(skip_all, fields(url = ?song_url.as_str()))]
    async fn get_download_url(song_url: &Url) -> anyhow::Result<String> {
        debug!("Getting song download URL");
        let download_id = Self::initialize_song_download(song_url).await?;
        Self::wait_for_song_to_finish(&download_id).await
    }

    async fn initialize_song_download(song_url: &Url) -> anyhow::Result<YamsId> {
        debug!("Initializing song download");

        let quality = match Self::get_quality(song_url) {
            Some(q) => q,
            None => anyhow::bail!("Could not determine quality to download"),
        };

        let payload = serde_json::json!({
            "url": song_url.as_str(),
            "quality": quality,
            "host": "filehaus",
        });

        trace!(
            ?payload,
            "Sending download request to music download service"
        );

        let resp = reqwest::Client::new()
            .post(API_URL)
            .json(&payload)
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?;

        trace!(?resp, "Response received from music download service");

        let resp_body = resp.text().await?;

        trace!(
            ?resp_body,
            "Response body received from music download service"
        );

        serde_json::from_str::<YamsInitialResponse>(&resp_body)
            .map(|x| x.id)
            .map_err(std::convert::Into::into)
    }

    fn get_quality(song_url: &Url) -> Option<&str> {
        QUALITY_MAP.iter().find_map(|(service, quality)| {
            song_url
                .host_str()
                .unwrap_or_default()
                .find(service)
                .map(|_| *quality)
        })
    }

    async fn wait_for_song_to_finish(download_id: &YamsId) -> anyhow::Result<String> {
        debug!("Waiting for song to finish");
        let mut api_url = Url::parse(API_URL).expect("Invalid API URL");
        let download_id = format!("{download_id}");
        api_url.query_pairs_mut().append_pair("id", &download_id);

        for _ in 0..300 {
            let resp = reqwest::Client::new()
                .get(api_url.as_str())
                .timeout(Duration::from_secs(5))
                .send()
                .await?
                .error_for_status()?
                .json::<YamsStatusResponse>()
                .await?;

            trace!(?resp, "Song download status");

            if let Some(err) = resp.error {
                anyhow::bail!(err);
            }

            if let Some(url) = resp.url {
                return Ok(url);
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        anyhow::bail!("Song download timed out");
    }
}
