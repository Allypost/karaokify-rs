mod handlers;

use std::path::{Path, PathBuf};

use handlers::HANDLERS;
use tracing::info;
use url::Url;

pub struct Downloader;
impl Downloader {
    #[tracing::instrument(skip_all, fields(url = ?song_url.as_str()))]
    pub async fn download_song(
        download_dir: &Path,
        song_url: &Url,
    ) -> Result<PathBuf, anyhow::Error> {
        info!("Downloading song...");

        for handler in HANDLERS.iter() {
            if !handler.supports(song_url).await {
                continue;
            }

            match handler.download(download_dir, song_url).await {
                Ok(path) => {
                    info!(?path, "Downloaded song");
                    return Ok(path);
                }
                Err(e) => {
                    info!(?e, ?handler, "Handler failed");
                    continue;
                }
            }
        }

        Err(anyhow::anyhow!(
            "No handler succeeded for provided URL: {song_url}"
        ))
    }
}
