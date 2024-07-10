use std::{
    ffi::OsString,
    fmt::Display,
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::process::Command;
use tracing::{debug, trace};

use crate::helpers::temp_dir::TempDir;

#[derive(Debug)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
pub enum DemucsModel {
    HTDemucs,
    HTDemucsFt,
    HTDemucs6s,
    HDemucsMmi,
    MDX,
    MDXExtra,
    MDXQ,
}
impl Display for DemucsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HTDemucs => f.write_str("htdemucs"),
            Self::HTDemucsFt => f.write_str("htdemucs_ft"),
            Self::HTDemucs6s => f.write_str("htdemucs_6s"),
            Self::HDemucsMmi => f.write_str("hdemucs_mmi"),
            Self::MDX => f.write_str("mdx"),
            Self::MDXExtra => f.write_str("mdx_extra"),
            Self::MDXQ => f.write_str("mdx_q"),
        }
    }
}

pub struct DemucsProcessor;
impl DemucsProcessor {
    #[tracing::instrument]
    pub async fn split_into_stems(
        output_dir: &Path,
        file_path: &Path,
        demucs_model: DemucsModel,
    ) -> anyhow::Result<Vec<PathBuf>> {
        debug!("Splitting into stems");
        let demucs_dir = TempDir::with_prefix("karaokify-demucs-").await?;

        let cmd_status = tryhard::retry_fn(|| {
            Command::new("demucs")
                .args(["--name", &demucs_model.to_string()])
                .args(["--two-stems", "vocals"])
                .args(["--filename", "{stem}.{ext}"])
                .args(["--mp3-bitrate", "256"])
                .arg("--mp3")
                .args([
                    OsString::from("--out").as_os_str(),
                    demucs_dir.path().as_os_str(),
                ])
                .arg(file_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status()
        })
        .retries(3)
        .await?;

        trace!(status = ?cmd_status, "Demucs command finished");

        if !cmd_status.success() {
            anyhow::bail!("Command executed with exit code {:?}", cmd_status.code());
        }

        let demucs_stems_dir = demucs_dir.path().join(demucs_model.to_string());

        let file_base_name = {
            let mut f = file_path.file_stem().unwrap_or_default().to_os_string();

            if f.is_empty() {
                f = OsString::from("song");
            }

            f
        };

        let vocals_path = output_dir.join({
            let mut f = file_base_name.clone();
            f.push(".vocals.mp3");
            f
        });
        trace!(?vocals_path, "Copying vocals to output directory");
        tokio::fs::copy(demucs_stems_dir.join("vocals.mp3"), &vocals_path).await?;

        let music_path = output_dir.join({
            let mut f = file_base_name.clone();
            f.push(".music.mp3");
            f
        });
        trace!(?music_path, "Copying music to output directory");
        tokio::fs::copy(demucs_stems_dir.join("no_vocals.mp3"), &music_path).await?;

        let music_with_vocals_path = output_dir.join({
            let mut f = file_base_name.clone();
            f.push(".music-with-quiet-vocals.mp3");
            f
        });
        let cmd_status = {
            let filter_cmd = "[0:a]volume=-20dB[voc];[voc][1:a]amix=inputs=2:duration=longest:\
                              dropout_transition=0:normalize=0";

            trace!("Combining vocals and music to create music with quiet vocals");
            Command::new("ffmpeg")
                .args([OsString::from("-i"), vocals_path.as_os_str().to_os_string()])
                .args([OsString::from("-i"), music_path.as_os_str().to_os_string()])
                .args(["-filter_complex", filter_cmd])
                .arg(demucs_stems_dir.join("music-with-quiet-vocals.mp3"))
                .args(["-b:a", "256k"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status()
                .await?
        };
        trace!(status = ?cmd_status, "Combine command finished");

        let mut files = vec![vocals_path, music_path];

        if cmd_status.success() {
            trace!(
                ?music_with_vocals_path,
                "Copying music with quiet vocal stems to output directory"
            );
            tokio::fs::copy(
                demucs_stems_dir.join("music-with-quiet-vocals.mp3"),
                &music_with_vocals_path,
            )
            .await?;

            files.push(music_with_vocals_path);
        }

        let mp3_file_path = file_path.with_extension("mp3");
        let cmd_status = {
            trace!("Re-encoding song to mp3");
            Command::new("ffmpeg")
                .args([OsString::from("-i"), file_path.as_os_str().to_os_string()])
                .args(["-b:a", "256k"])
                .arg(demucs_stems_dir.join("song.mp3"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status()
                .await?
        };
        trace!(status = ?cmd_status, "Re-encoding command finished");

        if cmd_status.success() {
            trace!(
                ?mp3_file_path,
                "Copying re-encoded song to output directory"
            );
            tokio::fs::copy(demucs_stems_dir.join("song.mp3"), &mp3_file_path).await?;

            files.push(mp3_file_path);
        }

        Ok(files)
    }
}
