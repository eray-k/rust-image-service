use anyhow::{anyhow, Result};
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader};
use std::path::Path;


/// Currently the crops will be centered in all cases.
pub struct OutputConfig {
    enforced_aspect_ratio: Option<f64>,
    resize_width: Option<u32>,
    pub convert_format: Option<ImageFormat>,
}

impl OutputConfig {
    pub const fn new() -> Self {
        Self {
            enforced_aspect_ratio: None,
            resize_width: None,
            convert_format: None,
        }
    }
    pub const fn aspect_ratio(mut self, val: f64) -> Self {
        self.enforced_aspect_ratio = Some(val);
        self
    }

    pub const fn rescale(mut self, width: u32) -> Self {
        self.resize_width = Some(width);
        self
    }

    pub const fn format(mut self, format: ImageFormat) -> Self {
        self.convert_format = Some(format);
        self
    }

    pub fn target_width(&self) -> Option<u32> { self.resize_width }
    pub fn target_height(&self) -> Option<u32> {
        let res = (self.resize_width? as f64) * self.enforced_aspect_ratio?.round();
        Some(res as u32)
    }
    pub fn target_aspect_ratio(&self) -> Option<f64> { self.enforced_aspect_ratio }
}

/// Checks if the file is a valid image file
///
/// # Returns
/// Error on unsupported image format.
/// Otherwise, educated guess of the image file.
///
pub fn validate_image<P: AsRef<Path>>(input: P) -> Result<ImageFormat> {
    let reader_with_format =
        ImageReader::open(input.as_ref())?.with_guessed_format()?;

    let format = reader_with_format.format().ok_or(anyhow!("Unsupported image format"))?;

    Ok(format)
}


/// Processes the given image
///
/// Currently, resizes (not preserving the aspect ratio) and converts the image into `webp` format.
pub fn process_image<P: AsRef<Path>>(input: P, output: P, output_config: &OutputConfig) -> Result<()> {
    let mut img = ImageReader::open(input.as_ref())?
        .with_guessed_format()?.decode()?;

    if let Some(ratio) = output_config.enforced_aspect_ratio {
        img = crop_image(&mut img, ratio);
    }

    // if let Some(width) = output_config.resize_width {
    //     img = resize_image(&mut img, width);
    // }

    match output_config.convert_format {
        Some(ImageFormat::WebP) => {
            let webp = match webp::Encoder::from_image(&img) {
                Ok(enc) => enc.encode(70f32),
                Err(e) => return Err(anyhow!("Error encoding image: {}", e)),
            };
            std::fs::write(output, webp.as_ref())?;
        }
        Some(format) => img.save_with_format(output, format)?,
        None => img.save(output)?,
    }

    Ok(())
}

fn crop_image(img: &mut DynamicImage, aspect_ratio: f64) -> DynamicImage {
    let (w,h) = img.dimensions();
    let image_aspect_ratio = w as f64 / h as f64;

    let fit_width = image_aspect_ratio < aspect_ratio;

    let (new_w, new_h) = match fit_width {
        true => (w, (w as f64 / aspect_ratio) as u32 ),
        false => ((h as f64 ) as u32, h ),
    };

    let x = (w - new_w)/2;
    let y = (h - new_h)/2;

    img.crop(x, y, new_w, new_h)
}

fn resize_image(img: &mut DynamicImage, resize_width: u32) -> DynamicImage {
    todo!()
}


#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageBuffer, Rgba, RgbaImage};
    use std::fs;
    use std::io::Read;
    use tempfile::tempdir;

    fn make_png(w: u32, h: u32) -> RgbaImage {
        // Simple gradient with alpha to exercise RGBA paths
        let mut img: RgbaImage = ImageBuffer::from_fn(w, h, |x, y| {
            let r = ((x * 255) / (w.max(1) - 1)).min(255) as u8;
            let g = ((y * 255) / (h.max(1) - 1)).min(255) as u8;
            let b = 180u8;
            let a = 255u8;
            Rgba([r, g, b, a])
        });
        // Ensure non-uniform pixels
        img.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        img
    }

    fn save_png<P: AsRef<Path>>(img: &RgbaImage, path: P) {
        img.save(path).expect("save png");
    }

    fn is_webp_file<P: AsRef<Path>>(path: P) -> bool {
        let mut f = fs::File::open(path).expect("open output");
        let mut header = [0u8; 12];
        f.read_exact(&mut header).expect("read header");
        // WebP is RIFF container: "RIFF" .... "WEBP"
        &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP"
    }

    // This test is no longer relevant
    // #[test]
    // fn converts_and_resizes_aspect_fit() {
    //     let dir = tempdir().unwrap();
    //     let in_path = dir.path().join("in.png");
    //     let out_path = dir.path().join("out.webp");
    //
    //     // Original 2000x1000 (2:1)
    //     let img = make_png(2000, 1000);
    //     save_png(&img, &in_path);
    //
    //     // Box 800x800 ⇒ expect 800x400 (aspect-fit, no upscale)
    //     process_image(&in_path, &out_path, ).unwrap();
    //     assert!(out_path.exists(), "webp not created");
    //     assert!(is_webp_file(&out_path), "not a WebP (bad header)");
    //
    //     // Decode and check dimensions
    //     let decoded = image::open(&out_path).expect("decode webp");
    //     let (w, h) = decoded.dimensions();
    //     assert_eq!((w, h), (800, 400), "unexpected resized size");
    // }
    #[test]
    fn produces_webp_file() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("small.png");
        let out_path = dir.path().join("small.webp");
        let config = OutputConfig::new()
            .format(ImageFormat::WebP);

        let img = make_png(320, 240);
        save_png(&img, &in_path);

        process_image(&in_path, &out_path, &config).unwrap();

        let reader = ImageReader::open(&out_path).expect("Can't access output image")
            .with_guessed_format().expect("Can't decode image");

        assert_eq!(reader.format().expect("Can't determine format"), ImageFormat::WebP);
    }
    #[test]
    fn applies_aspect_ratio() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("small.png");
        let out_path = dir.path().join("small.webp");
        let config = OutputConfig::new()
            .aspect_ratio(1.);

        let img = make_png(320, 240);
        save_png(&img, &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).expect("decode webp");
        let (w, h) = decoded.dimensions();
        assert_eq!((w, h), (240, 240), "image isn't the correct size");
    }

    // #[test]
    // fn no_upscale_when_smaller_than_box() {
    //     let dir = tempdir().unwrap();
    //     let in_path = dir.path().join("small.png");
    //     let out_path = dir.path().join("small.webp");
    //
    //     // Original 320x240, box 800x800 ⇒ should remain 320x240
    //     let img = make_png(320, 240);
    //     save_png(&img, &in_path);
    //
    //     process_image(&in_path, &out_path).unwrap();
    //     let decoded = image::open(&out_path).expect("decode webp");
    //     let (w, h) = decoded.dimensions();
    //     assert_eq!((w, h), (320, 240), "image was upscaled but shouldn't be");
    // }
    //
    // #[test]
    // fn wide_box_clamps_by_height() {
    //     let dir = tempdir().unwrap();
    //     let in_path = dir.path().join("in2.png");
    //     let out_path = dir.path().join("out2.webp");
    //
    //     // Original 1200x1600 (portrait)
    //     let img = make_png(1200, 1600);
    //     save_png(&img, &in_path);
    //
    //     // Box 1000x600 ⇒ height is limiting ⇒ 450x600
    //     process_image(&in_path, &out_path).unwrap();
    //     let decoded = image::open(&out_path).expect("decode webp");
    //     let (w, h) = decoded.dimensions();
    //     assert_eq!((w, h), (450, 600));
    // }

    // #[test]
    // fn maintains_alpha_channel_after_encode() {
    //     let dir = tempdir().unwrap();
    //     let in_path = dir.path().join("alpha.png");
    //     let out_path = dir.path().join("alpha.webp");
    //
    //     // 100x100 with a transparent stripe
    //     let mut img = RgbaImage::new(100, 100);
    //     for y in 0..100 {
    //         for x in 0..100 {
    //             let a = if x < 50 { 0 } else { 255 };
    //             img.put_pixel(x, y, Rgba([10, 200, 100, a]));
    //         }
    //     }
    //     save_png(&img, &in_path);
    //
    //     process_image(&in_path, &out_path).unwrap();
    //     let decoded = image::open(&out_path).unwrap().to_rgba8();
    //
    //     // Check that some pixels remain transparent and some opaque
    //     let mut saw_transparent = false;
    //     let mut saw_opaque = false;
    //     for y in 0..decoded.height() {
    //         for x in 0..decoded.width() {
    //             let a = decoded.get_pixel(x, y)[3];
    //             if a < 20 { saw_transparent = true; }
    //             if a > 230 { saw_opaque = true; }
    //         }
    //     }
    //     assert!(saw_transparent && saw_opaque, "alpha lost during conversion");
    // }
}
