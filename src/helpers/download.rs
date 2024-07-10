use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::{header, Response};
use tokio::{
    fs,
    io::{AsyncWriteExt, BufWriter},
};
use tracing::{debug, trace};

use super::header::content_disposition::ContentDisposition;
use crate::helpers::temp_file::TempFile;

#[tracing::instrument]
pub async fn download_file(download_path: &Path, download_url: &str) -> anyhow::Result<PathBuf> {
    let resp = get_file_response(download_url).await?;

    write_resp_to_file(resp, download_path).await
}

/// Infer file name from content disposition if present, else use provided path
#[tracing::instrument]
pub async fn download_file_inferred(
    download_path: &Path,
    download_url: &str,
) -> anyhow::Result<PathBuf> {
    let resp = get_file_response(download_url).await?;

    let content_disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|x| ContentDisposition::from_raw(x).ok());

    let filename = content_disposition
        .and_then(|x| x.get_filename().map(PathBuf::from))
        .and_then(|x| x.file_name().map(std::ffi::OsStr::to_os_string));

    let download_path = filename.map_or_else(
        || download_path.to_path_buf(),
        |filename| download_path.with_file_name(filename),
    );

    write_resp_to_file(resp, &download_path).await
}

async fn get_file_response(download_url: &str) -> anyhow::Result<Response> {
    debug!("Starting download");
    reqwest::Client::new()
        .get(download_url)
        .timeout(Duration::from_secs(60))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!(e))
}

async fn write_resp_to_file(mut resp: Response, file_path: &Path) -> anyhow::Result<PathBuf> {
    trace!(path = ?file_path, "Writing request response to disk");

    let mut temp_file = TempFile::with_prefix("karaokify-download-").await?;
    trace!(f = ?temp_file.path(), "Created temp file for download");

    {
        let mut out_file = BufWriter::new(temp_file.file_mut());

        while let Some(chunk) = resp.chunk().await? {
            out_file.write_all(&chunk).await?;
        }
        out_file.flush().await?;
    }
    trace!("Finished writing to disk");

    trace!(from = ?temp_file.path(), to = ?file_path, "Copying temp file to download path");
    fs::copy(temp_file.path(), &file_path).await?;

    debug!("Response written to disk");

    Ok(file_path.to_path_buf())
}
