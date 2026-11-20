use std::path::PathBuf;

use axum::{extract::{Path, State}, http::{header, HeaderValue, StatusCode}, response::{IntoResponse, Response}};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tokio_util::io::ReaderStream;
use tempfile::NamedTempFile;
use tokio::fs::File;
use tokio::sync::OwnedSemaphorePermit;
use tracing::{error, warn};
use uuid::Uuid;

use crate::{auth::AuthUser, config::CONFIG, error::handle_internal_error, AppState};
use crate::engine::image_engine::{process_image, validate_image, ImageInfo, InvalidImage};
use crate::runtime::{Runtime, RuntimeConfig};

pub use image_engine::OutputConfig;

mod image_engine;

/// Estimated peak memory of processing an image in MiB: the decoded buffer and its
/// cropped copy at the input's pixel size, plus the 8-bit copy made before encoding.
fn processing_cost_mb(info: &ImageInfo, budget_mb: usize) -> u32 {
    let bytes = info.width as u64 * info.height as u64 * (2 * info.bytes_per_pixel as u64 + 4);
    // An image costing more than the whole budget still runs, alone.
    (bytes.div_ceil(1024 * 1024) as u32).clamp(1, budget_mb as u32)
}

/// Waits for memory budget and a worker slot to process the image.
async fn acquire_processing(
    runtime: &Runtime,
    config: &RuntimeConfig,
    info: &ImageInfo,
) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), (StatusCode, String)> {
    // Worker first, so memory is only reserved by jobs that are ready to run
    let acquire = async {
        let worker = runtime.workers.acquire(1).await?;
        let cost = processing_cost_mb(info, runtime.memory_budget.size());
        let memory = runtime.memory_budget.acquire(cost).await?;
        Ok::<_, tokio::sync::AcquireError>((memory, worker))
    };

    match tokio::time::timeout(config.queue_timeout(), acquire).await {
        Ok(permits) => permits.map_err(handle_internal_error),
        Err(_) => Err((StatusCode::SERVICE_UNAVAILABLE, "Server is busy, try again later".to_string())),
    }
}

#[derive(TryFromMultipart)]
pub struct UploadFileRequest {
    #[form_data(limit = "2MiB")]
    image: FieldData<NamedTempFile>,
}

pub async fn upload(
    state: State<AppState>,
    AuthUser(user_id): AuthUser,
    TypedMultipart(UploadFileRequest {
        image,
    }): TypedMultipart<UploadFileRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let info = validate_image(image.contents.path()).map_err(|e| {
        warn!("Validation error: {}", e);
        (StatusCode::BAD_REQUEST, "Invalid image file".to_string())
    })?;

    // One snapshot for the whole request, so a reload midway can't mix settings
    let config = state.runtime.config();
    let format = config.image_format();
    let output_config = config.output_config();

    let new_uuid = uuid::Uuid::new_v4();

    let mut filepath = PathBuf::from(&CONFIG.upload_dir);
    filepath.push(new_uuid.to_string());
    filepath.set_extension(format.extensions_str()[0]);

    // Process and persist image before recording it, so a failure leaves no dangling row
    let output_path = filepath.clone();
    let permits = acquire_processing(&state.runtime, &config, &info).await?;
    tokio::task::spawn_blocking(move || {
        // Held by the task itself, so capacity stays reserved even if the client disconnects
        let _permits = permits;
        process_image(image.contents.path(), &output_path, &output_config)
    }).await
        .map_err(handle_internal_error)?
        .map_err(|e| match e.downcast_ref::<InvalidImage>() {
            // Header looked fine but the content isn't a decodable image
            Some(_) => {
                warn!("Validation error: {}", e);
                (StatusCode::BAD_REQUEST, "Invalid image file".to_string())
            }
            None => handle_internal_error(e),
        })?;

    // Insert into db
    if let Err(e) = sqlx::query!(
        "insert into image (id, user_id, filepath, mime) values ($1, $2, $3, $4)",
        new_uuid,
        user_id,
        filepath.to_str(),
        format.to_mime_type()
    )
    .execute(&state.db)
    .await {
        let _ = tokio::fs::remove_file(&filepath).await;
        return Err(handle_internal_error(e));
    }

    Ok((StatusCode::CREATED, new_uuid.to_string()))
}

pub async fn download(
    Path(image_id): Path<Uuid>,
    state: State<AppState>) -> Result<Response, (StatusCode, String)> {
    let not_found_err = Err((StatusCode::NOT_FOUND, "File not found".to_string()));

    let db_model = match sqlx::query!("select user_id, filepath, mime from image where id=$1", image_id)
        .fetch_optional(&state.db).await {
            Ok(Some(model)) => model,
            Ok(None) => return not_found_err,
            Err(e) => return Err(handle_internal_error(e))
        };

    if db_model.filepath.is_none() || db_model.mime.is_none() {
        error!("Database model is missing fields: {:?}", db_model);
        return not_found_err;
    }

    let path = PathBuf::from(db_model.filepath.unwrap());

    // Ensure file exists
    if !path.is_file() {
        error!("File does not exist at path: {:?}", path);
        return not_found_err;
    }

    // Open & stream
    let file = match File::open(&path).await {
            Ok(f) => f,
            Err(e) => return Err(handle_internal_error(e))
    };

    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    // Content-Disposition (RFC 5987 for UTF-8 filenames)
    let encoded = utf8_percent_encode(&image_id.to_string(), NON_ALPHANUMERIC).to_string();
    let cd_value = format!("attachment; filename*=UTF-8''{encoded}");

    let mut res = Response::new(body);
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&db_model.mime.unwrap()).unwrap());
    headers.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&cd_value).unwrap());

    Ok(res)
}

pub async fn delete_image(
    Path(image_id): Path<Uuid>,
    state: State<AppState>,
    AuthUser(user_id): AuthUser,
) -> Result<(), (StatusCode, String)> {
    // Someone else's image is reported as not found, so ids of other users' images aren't revealed
    let filepath = sqlx::query_scalar!(
        "delete from image where id=$1 and user_id=$2 returning filepath",
        image_id,
        user_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(handle_internal_error)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Image not found".to_string()))?;

    // Row is already gone; a failure here only leaks the file
    if let Some(filepath) = filepath
        && let Err(e) = tokio::fs::remove_file(&filepath).await {
        error!("Failed to delete file {}: {}", filepath, e);
    }

    Ok(())
}
