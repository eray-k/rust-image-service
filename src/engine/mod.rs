use axum::{extract::{Path, State}, http::{header, HeaderValue, StatusCode}, response::{IntoResponse, Response}};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use object_store::{path::Path as StorePath, ObjectStoreExt, PutPayload};
use tempfile::NamedTempFile;
use tokio::sync::OwnedSemaphorePermit;
use tracing::{error, warn};
use uuid::Uuid;

use crate::{auth::AuthUser, error::handle_internal_error, AppState};
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

    let key = format!("{}.{}", new_uuid, format.extensions_str()[0]);
    let key_path = StorePath::from(key.as_str());

    // Process into a temp file first; it's only sent to storage once it's known to be valid
    let output = tempfile::Builder::new()
        .suffix(&format!(".{}", format.extensions_str()[0]))
        .tempfile()
        .map_err(handle_internal_error)?;
    let permits = acquire_processing(&state.runtime, &config, &info).await?;
    let output = tokio::task::spawn_blocking(move || {
        // Held by the task itself, so capacity stays reserved even if the client disconnects
        let _permits = permits;
        process_image(image.contents.path(), output.path(), &output_config).map(|_| output)
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

    // Output is capped by the 8192 px limit and the request size, so it fits in memory
    let bytes = tokio::fs::read(output.path()).await.map_err(handle_internal_error)?;

    // Store before recording, so a failure leaves no dangling row
    state.storage.put(&key_path, PutPayload::from(bytes)).await.map_err(handle_internal_error)?;

    // Insert into db
    if let Err(e) = sqlx::query!(
        "insert into image (id, user_id, storage_key, mime) values ($1, $2, $3, $4)",
        new_uuid,
        user_id,
        key,
        format.to_mime_type()
    )
    .execute(&state.db)
    .await {
        if let Err(e) = state.storage.delete(&key_path).await {
            error!("Failed to remove orphaned object {}: {}", key, e);
        }
        return Err(handle_internal_error(e));
    }

    Ok((StatusCode::CREATED, new_uuid.to_string()))
}

pub async fn download(
    Path(image_id): Path<Uuid>,
    state: State<AppState>) -> Result<Response, (StatusCode, String)> {
    let not_found_err = Err((StatusCode::NOT_FOUND, "File not found".to_string()));

    let db_model = match sqlx::query!("select user_id, storage_key, mime from image where id=$1", image_id)
        .fetch_optional(&state.db).await {
            Ok(Some(model)) => model,
            Ok(None) => return not_found_err,
            Err(e) => return Err(handle_internal_error(e))
        };

    if db_model.storage_key.is_none() || db_model.mime.is_none() {
        error!("Database model is missing fields: {:?}", db_model);
        return not_found_err;
    }

    let key = db_model.storage_key.unwrap();
    let object = match state.storage.get(&StorePath::from(key.as_str())).await {
        Ok(object) => object,
        Err(object_store::Error::NotFound { .. }) => {
            error!("Object does not exist: {}", key);
            return not_found_err;
        }
        Err(e) => return Err(handle_internal_error(e)),
    };

    let body = axum::body::Body::from_stream(object.into_stream());

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
    let storage_key = sqlx::query_scalar!(
        "delete from image where id=$1 and user_id=$2 returning storage_key",
        image_id,
        user_id
    )
    .fetch_optional(&state.db)
    .await
    .map_err(handle_internal_error)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Image not found".to_string()))?;

    // Row is already gone; a failure here only leaks the object
    if let Some(key) = storage_key
        && let Err(e) = state.storage.delete(&StorePath::from(key.as_str())).await
        && !matches!(e, object_store::Error::NotFound { .. }) {
        error!("Failed to delete object {}: {}", key, e);
    }

    Ok(())
}
