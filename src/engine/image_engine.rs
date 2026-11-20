use anyhow::{anyhow, Result};
use image::{imageops::FilterType, DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::path::Path;

/// Largest accepted width/height, so a small upload can't declare huge dimensions (decompression bomb).
pub const MAX_DIMENSION: u32 = 8192;


/// The uploaded file could not be decoded, as opposed to a failure on our side.
#[derive(Debug)]
pub struct InvalidImage(pub image::ImageError);

impl std::fmt::Display for InvalidImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid image: {}", self.0)
    }
}

impl std::error::Error for InvalidImage {}

/// Currently the crops will be centered in all cases.
pub struct OutputConfig {
    enforced_aspect_ratio: Option<f64>,
    resize_width: Option<u32>,
    pub convert_format: Option<ImageFormat>,
    webp_quality: f32,
}

impl OutputConfig {
    pub const fn new() -> Self {
        Self {
            enforced_aspect_ratio: None,
            resize_width: None,
            convert_format: None,
            webp_quality: 70.,
        }
    }
    pub const fn aspect_ratio(mut self, val: f64) -> Self {
        self.enforced_aspect_ratio = Some(val);
        self
    }

    /// Downscales to `width`, keeping the aspect ratio. Smaller images are not upscaled.
    pub const fn rescale(mut self, width: u32) -> Self {
        self.resize_width = Some(width);
        self
    }

    pub const fn format(mut self, format: ImageFormat) -> Self {
        self.convert_format = Some(format);
        self
    }

    /// Lossy WebP quality, 0 to 100.
    pub const fn webp_quality(mut self, quality: f32) -> Self {
        self.webp_quality = quality;
        self
    }
}

/// What the header of a valid image declares.
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// Bytes per pixel of the decoded buffer, e.g. 8 for 16-bit RGBA
    pub bytes_per_pixel: u8,
}

/// Checks if the file is a valid image file, reading only its header
///
/// # Returns
/// Error on unsupported image format.
/// Otherwise, the image dimensions and decoded pixel size.
///
pub fn validate_image<P: AsRef<Path>>(input: P) -> Result<ImageInfo> {
    let reader_with_format =
        ImageReader::open(input.as_ref())?.with_guessed_format()?;

    if reader_with_format.format().is_none() {
        return Err(anyhow!("Unsupported image format"));
    }

    let decoder = reader_with_format.into_decoder()?;
    let (w, h) = decoder.dimensions();
    if w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err(anyhow!("Image dimensions {w}x{h} exceed {MAX_DIMENSION}"));
    }

    Ok(ImageInfo { width: w, height: h, bytes_per_pixel: decoder.color_type().bytes_per_pixel() })
}


/// Processes the given image
///
/// Crops to the configured aspect ratio, downscales to the configured width, then converts to the configured format.
pub fn process_image<P: AsRef<Path>>(input: P, output: P, output_config: &OutputConfig) -> Result<()> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);

    let mut reader = ImageReader::open(input.as_ref())?.with_guessed_format()?;
    reader.limits(limits);
    let mut img = reader.decode().map_err(InvalidImage)?;

    if let Some(ratio) = output_config.enforced_aspect_ratio {
        img = crop_image(&mut img, ratio);
    }

    if let Some(width) = output_config.resize_width {
        img = resize_image(img, width);
    }

    match output_config.convert_format {
        Some(ImageFormat::WebP) => {
            // libwebp only accepts 8-bit RGB/RGBA, so normalize grayscale and 16-bit inputs.
            let img = match img.color().has_alpha() {
                true => DynamicImage::ImageRgba8(img.to_rgba8()),
                false => DynamicImage::ImageRgb8(img.to_rgb8()),
            };
            let webp = match webp::Encoder::from_image(&img) {
                Ok(enc) => enc.encode(output_config.webp_quality),
                Err(e) => return Err(anyhow!("Error encoding image: {}", e)),
            };
            std::fs::write(output, &*webp)?;
        }
        // JPEG has no alpha channel
        Some(ImageFormat::Jpeg) => DynamicImage::ImageRgb8(img.to_rgb8()).save_with_format(output, ImageFormat::Jpeg)?,
        Some(format) => img.save_with_format(output, format)?,
        None => img.save(output)?,
    }

    Ok(())
}

fn crop_image(img: &mut DynamicImage, aspect_ratio: f64) -> DynamicImage {
    let (w,h) = img.dimensions();
    let image_aspect_ratio = w as f64 / h as f64;

    let fit_width = image_aspect_ratio < aspect_ratio;

    // At least 1px, so an extreme ratio on a tiny image can't crop it away entirely
    let (new_w, new_h) = match fit_width {
        true => (w, ((w as f64 / aspect_ratio) as u32).max(1)),
        false => (((h as f64 * aspect_ratio) as u32).max(1), h),
    };

    let x = (w - new_w)/2;
    let y = (h - new_h)/2;

    img.crop(x, y, new_w, new_h)
}

fn resize_image(img: DynamicImage, resize_width: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w <= resize_width {
        return img;
    }

    let new_h = ((h as f64 * resize_width as f64 / w as f64).round() as u32).max(1);
    img.resize_exact(resize_width, new_h, FilterType::Lanczos3)
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

    #[test]
    fn converts_and_resizes_keeping_aspect() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("in.png");
        let out_path = dir.path().join("out.webp");
        let config = OutputConfig::new()
            .rescale(800)
            .format(ImageFormat::WebP);

        save_png(&make_png(2000, 1000), &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        assert!(is_webp_file(&out_path), "not a WebP (bad header)");
        let decoded = image::open(&out_path).expect("decode webp");
        assert_eq!(decoded.dimensions(), (800, 400), "unexpected resized size");
    }

    #[test]
    fn no_upscale_when_smaller_than_width() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("small.png");
        let out_path = dir.path().join("small_out.png");
        let config = OutputConfig::new()
            .rescale(800);

        save_png(&make_png(320, 240), &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).expect("decode png");
        assert_eq!(decoded.dimensions(), (320, 240), "image was upscaled but shouldn't be");
    }

    #[test]
    fn crops_then_resizes() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("in.png");
        let out_path = dir.path().join("out.png");
        let config = OutputConfig::new()
            .aspect_ratio(1.)
            .rescale(100);

        save_png(&make_png(1200, 1600), &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).expect("decode png");
        assert_eq!(decoded.dimensions(), (100, 100));
    }

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

    #[test]
    fn applies_non_square_aspect_ratio() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("wide.png");
        let out_path = dir.path().join("wide_out.png");
        let config = OutputConfig::new()
            .aspect_ratio(2.);

        save_png(&make_png(1000, 200), &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).expect("decode png");
        assert_eq!(decoded.dimensions(), (400, 200), "image isn't the correct size");
    }

    #[test]
    fn encodes_grayscale_as_webp() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("gray.png");
        let out_path = dir.path().join("gray.webp");
        let config = OutputConfig::new()
            .format(ImageFormat::WebP);

        image::GrayImage::new(64, 64).save(&in_path).unwrap();

        process_image(&in_path, &out_path, &config).unwrap();
        assert!(is_webp_file(&out_path), "not a WebP (bad header)");
    }

    #[test]
    fn maintains_alpha_channel_after_encode() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("alpha.png");
        let out_path = dir.path().join("alpha.webp");
        let config = OutputConfig::new()
            .format(ImageFormat::WebP);

        // 100x100 with a transparent stripe
        let img = RgbaImage::from_fn(100, 100, |x, _| {
            Rgba([10, 200, 100, if x < 50 { 0 } else { 255 }])
        });
        save_png(&img, &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).unwrap().to_rgba8();

        let saw_transparent = decoded.pixels().any(|p| p[3] < 20);
        let saw_opaque = decoded.pixels().any(|p| p[3] > 230);
        assert!(saw_transparent && saw_opaque, "alpha lost during conversion");
    }

    #[test]
    fn rejects_disabled_format() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("in.gif");
        // Valid GIF magic bytes, but the GIF decoder is not compiled in
        std::fs::write(&path, b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").unwrap();

        assert!(validate_image(&path).is_err());
    }

    #[test]
    fn reports_decoded_pixel_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("deep.png");
        image::ImageBuffer::<Rgba<u16>, _>::new(4, 2).save(&path).unwrap();

        let info = validate_image(&path).unwrap();
        assert_eq!((info.width, info.height, info.bytes_per_pixel), (4, 2, 8));
    }

    #[test]
    fn extreme_aspect_ratio_keeps_one_pixel() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("tiny.png");
        let out_path = dir.path().join("tiny.webp");
        let config = OutputConfig::new()
            .aspect_ratio(2.)
            .format(ImageFormat::WebP);

        save_png(&RgbaImage::new(1, 1), &in_path);

        process_image(&in_path, &out_path, &config).unwrap();
        let decoded = image::open(&out_path).expect("decode webp");
        assert_eq!(decoded.dimensions(), (1, 1));
    }
}
