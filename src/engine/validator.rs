use std::error::Error;

use tempfile::NamedTempFile;

use crate::image_type::{FindFileTypeExt, ImageType};

pub trait ValidateExt {
    type Error;

    async fn validate(&mut self) -> Result<ImageType, Self::Error>;
}

impl ValidateExt for NamedTempFile {
    type Error = Box<dyn Error>;

    async fn validate(&mut self) -> Result<ImageType, Self::Error> {
        let filetype = self.as_file_mut().find_imagetype().await?; // TODO: handle error

        Ok(filetype)
    }
}

