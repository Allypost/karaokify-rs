pub(super) mod spotifydown;
pub(super) mod yams;

use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use url::Url;

pub static HANDLERS: Lazy<Vec<DownloadHandler>> = Lazy::new(|| {
    vec![
        DownloadHandler::new(yams::YamsProvider),
        DownloadHandler::new(spotifydown::SpotifydownProvider),
    ]
});

#[derive(Debug)]
pub struct DownloadHandler {
    provider: Box<dyn Handler>,
}
impl DownloadHandler {
    fn new<T>(provider: T) -> Self
    where
        T: Handler + 'static,
    {
        Self {
            provider: Box::new(provider),
        }
    }

    pub async fn supports(&self, url: &Url) -> bool {
        self.provider.supports(url).await
    }

    pub async fn download(&self, download_dir: &Path, url: &Url) -> Result<PathBuf, anyhow::Error> {
        self.provider.download(download_dir, url).await
    }
}

#[async_trait::async_trait]
pub trait Handler: std::fmt::Debug + Send + Sync {
    async fn download(&self, download_dir: &Path, song_url: &Url) -> anyhow::Result<PathBuf>;

    async fn supports(&self, song_url: &Url) -> bool;
}
