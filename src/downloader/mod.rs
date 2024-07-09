mod handlers;

use std::path::{Path, PathBuf};

use handlers::HANDLERS;
use tracing::info;
use url::Url;

pub struct Downloader;
impl Downloader {
    pub async fn download_song(
        download_dir: &Path,
        song_url: &Url,
    ) -> Result<PathBuf, anyhow::Error> {
        info!(url = ?song_url.as_str(), "Downloading song...");

        for provider in HANDLERS.iter() {
            if provider.supports(song_url).await {
                return provider.download(download_dir, song_url).await;
            }
        }

        Err(anyhow::anyhow!("No provider for provided URL found"))
    }
}
