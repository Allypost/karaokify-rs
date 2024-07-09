use std::{
    env,
    ffi::OsString,
    marker::Send,
    path::{Path, PathBuf},
};

use tokio::fs;

use super::id::time_thread_id;

pub struct TempDir {
    path: PathBuf,
    delete_on_drop: bool,
}
impl TempDir {
    pub async fn new<T>(dir_name: T) -> Result<Self, std::io::Error>
    where
        T: Into<OsString> + Send,
    {
        let tmp_dir = env::temp_dir();
        let tmp_dir = tmp_dir.join(dir_name.into());

        fs::create_dir_all(&tmp_dir).await?;

        Ok(Self {
            path: tmp_dir,
            delete_on_drop: true,
        })
    }

    pub async fn with_prefix<T>(dir_name_prefix: T) -> Result<Self, std::io::Error>
    where
        T: Into<OsString> + Send,
    {
        let mut f: OsString = dir_name_prefix.into();
        f.push(time_thread_id());
        Self::new(f).await
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[allow(dead_code)]
    pub fn no_delete_on_drop(&mut self) -> &mut Self {
        self.delete_on_drop = false;
        self
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.delete_on_drop {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
