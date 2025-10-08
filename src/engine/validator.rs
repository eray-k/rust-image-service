use tempfile::NamedTempFile;

use crate::image_type::{FindFileTypeExt, ImageType};

pub struct ValidatedFile {
    pub filetype: ImageType,
    pub file: NamedTempFile,
}

impl TryFrom<NamedTempFile> for ValidatedFile {
    type Error = Box<dyn std::error::Error>;

    fn try_from(mut value: NamedTempFile) -> Result<Self, Self::Error> {
        let filetype = value.find_imagetype()?;

        Ok(ValidatedFile {
            filetype,
            file: value,
        })
    }
}
