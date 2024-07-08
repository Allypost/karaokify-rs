mod song_downloader;

use std::path::{Path, PathBuf};

use song_downloader::SongDownload;
use tracing::info;
use url::Url;

pub struct Downloader;
impl Downloader {
    pub async fn download_song(
        download_dir: &Path,
        song_url: &Url,
    ) -> Result<PathBuf, anyhow::Error> {
        info!(?song_url, "Downloading song...");

        let song_file_path = SongDownload::download_song(download_dir, song_url).await?;

        Ok(song_file_path)
    }
}
