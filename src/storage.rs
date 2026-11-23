use std::sync::Arc;

use anyhow::{Context, Result};
use object_store::{aws::AmazonS3Builder, local::LocalFileSystem, ObjectStore};

use crate::config::CONFIG;

/// Where processed images are kept: `UPLOAD_DIR` on disk, or an S3-compatible bucket when
/// `S3_BUCKET` is set. Objects are addressed by key (`<uuid>.<ext>`), which is what the
/// `image.filepath` column holds.
pub fn init() -> Result<Arc<dyn ObjectStore>> {
    match &CONFIG.s3_bucket {
        // Credentials, region and endpoint come from the AWS_* variables (AWS_ENDPOINT and
        // AWS_ALLOW_HTTP for MinIO and the like)
        Some(bucket) => Ok(Arc::new(
            AmazonS3Builder::from_env()
                .with_bucket_name(bucket)
                .build()
                .context("Couldn't configure S3 bucket")?,
        )),
        None => {
            std::fs::create_dir_all(&CONFIG.upload_dir).context("Couldn't create upload directory")?;
            Ok(Arc::new(
                LocalFileSystem::new_with_prefix(&CONFIG.upload_dir).context("Couldn't open upload directory")?,
            ))
        }
    }
}
