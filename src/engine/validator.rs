use tempfile::NamedTempFile;
use anyhow::Result;

use crate::image_type::{FindFileTypeExt, ImageType};

pub trait ValidateExt {
    async fn validate(&mut self) -> Result<ImageType>;
}

impl ValidateExt for NamedTempFile {

    async fn validate(&mut self) -> Result<ImageType> {
        let filetype = self.as_file_mut().find_imagetype().await?;

        Ok(filetype)
    }
}

