use std::io::Read;

use tempfile::NamedTempFile;

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

    pub fn extension(&self) -> &'static str {
        match self {
            ImageType::Jpeg => "jpg",
            ImageType::Png => "png",
            ImageType::Webp => "webp",
        }
    }
}

pub trait FindFileTypeExt {
    fn find_imagetype(&mut self) -> Result<ImageType, Box<dyn std::error::Error>>;
}

impl FindFileTypeExt for NamedTempFile {
    fn find_imagetype(&mut self) -> Result<ImageType, Box<dyn std::error::Error>> {
        let mut buf: [u8; 12] = [0; 12];
        self.read(&mut buf).unwrap();
        println!("{:?}", buf);

        match buf {
            [0x89, 0x50, 0x4e, 0x47, ..] => Ok(ImageType::Png),
            [0xff, 0xd8, 0xff, 0xe0, ..] => Ok(ImageType::Jpeg), // TODO: Also check for EXIF format

            // Source: https://www.iana.org/assignments/media-types/image/webp
            [0x52, 0x49, 0x46, 0x46, .., 0x57, 0x45, 0x42, 0x50] => Ok(ImageType::Webp),
            _ => Err("unsupported file type".into()),
        }
    }
}
