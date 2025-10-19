use std::{fs::File, io::Read};

use anyhow::Result;

pub enum ImageType {
    Jpeg,
    Png,
    Webp,
}

impl ImageType {
    // pub fn from_mime(mime: &str) -> Option<Self> {
    //     match mime {
    //         "image/jpeg" => Some(Self::Jpeg),
    //         "image/png" => Some(Self::Png),
    //         "image/webp" => Some(Self::Webp),
    //         _ => None,
    //     }
    // }

    pub fn to_mime(&self) -> &'static str {
        match self {
            ImageType::Jpeg => "image/jpeg",
            ImageType::Png => "image/png",
            ImageType::Webp => "image/webp",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ImageType::Jpeg => "jpg",
            ImageType::Png => "png",
            ImageType::Webp => "webp",
        }
    }
}

pub trait FindFileTypeExt {
    async fn find_imagetype(&mut self) -> Result<ImageType>;
}

impl FindFileTypeExt for File {
    async fn find_imagetype(&mut self) -> Result<ImageType> {
        let mut self_clone = self.try_clone()?; // Needed for lifetime
        let buf = tokio::task::spawn_blocking(move || -> Result<[u8;12], std::io::Error> {
            let mut buf: [u8; 12] = [0; 12];
            self_clone.read(&mut buf)?;
            Ok(buf)
        }).await??;

        match buf {
            [0x89, 0x50, 0x4e, 0x47, ..] => Ok(ImageType::Png),
            [0xff, 0xd8, 0xff, 0xe0, ..] => Ok(ImageType::Jpeg), // TODO: Also check for EXIF format

            // Source: https://www.iana.org/assignments/media-types/image/webp
            [0x52, 0x49, 0x46, 0x46, .., 0x57, 0x45, 0x42, 0x50] => Ok(ImageType::Webp),
            _ => Err(anyhow::anyhow!("Unsupported file type")),
        }
    }
}
