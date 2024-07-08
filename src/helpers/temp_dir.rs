use std::{
    env,
    ffi::OsString,
    fs::{self},
    path::{Path, PathBuf},
};

use super::id::time_thread_id;

pub struct TempDir {
    path: PathBuf,
    delete_on_drop: bool,
}
impl TempDir {
    pub fn new<T: Into<OsString>>(dir_name: T) -> Result<Self, std::io::Error> {
        let tmp_dir = env::temp_dir();
        let tmp_dir = tmp_dir.join(dir_name.into());

        std::fs::create_dir_all(&tmp_dir)?;

        Ok(Self {
            path: tmp_dir,
            delete_on_drop: true,
        })
    }

    pub fn with_prefix<T: Into<OsString>>(dir_name_prefix: T) -> Result<Self, std::io::Error> {
        let mut f: OsString = dir_name_prefix.into();
        f.push(time_thread_id());
        Self::new(f)
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
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
