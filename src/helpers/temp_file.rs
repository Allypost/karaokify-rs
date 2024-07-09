use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use tokio::fs::File;

use super::id::time_thread_id;

pub struct TempFile {
    path: PathBuf,
    file: File,
    delete_on_drop: bool,
}
impl TempFile {
    pub async fn new<T>(file_name: T) -> Result<Self, std::io::Error>
    where
        T: Into<OsString> + std::marker::Send,
    {
        let tmp_dir = std::env::temp_dir();
        let tmp_file = tmp_dir.join(file_name.into());
        let file = File::create(&tmp_file).await?;

        Ok(Self {
            path: tmp_file,
            file,
            delete_on_drop: true,
        })
    }

    pub async fn with_prefix<T>(file_name_prefix: T) -> Result<Self, std::io::Error>
    where
        T: Into<OsString> + std::marker::Send,
    {
        let mut f: OsString = file_name_prefix.into();
        f.push(time_thread_id());
        Self::new(f).await
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    #[allow(dead_code)]
    pub fn no_delete_on_drop(&mut self) -> &mut Self {
        self.delete_on_drop = false;
        self
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.delete_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
