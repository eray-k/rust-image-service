use std::path::PathBuf;

use axum::{extract::{Path, State}, http::{header, HeaderValue, StatusCode}, response::{IntoResponse, Response}};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use image::ImageFormat;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tokio_util::io::ReaderStream;
use tempfile::NamedTempFile;
use tokio::fs::File;
use uuid::Uuid;

use crate::{config::CONFIG, error::handle_internal_error, AppState};
use crate::engine::image_engine::{process_image, validate_image, OutputConfig};

mod image_engine;

const OUTPUT_CONFIG: OutputConfig = OutputConfig::new()
    .aspect_ratio(1.0)
    .format(ImageFormat::WebP);

#[derive(TryFromMultipart)]
pub struct UploadFileRequest {
    user_id: Uuid,
    #[form_data(limit = "2MiB")]
    image: FieldData<NamedTempFile>,
}

pub async fn upload(
    state: State<AppState>,
    TypedMultipart(UploadFileRequest {
        user_id,
        image,
    }): TypedMultipart<UploadFileRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let _ = validate_image(&image.contents.path()).map_err(|e| {
        eprintln!("Validation error: {}", e);
        (StatusCode::BAD_REQUEST, "Invalid image file".to_string())
    })?;

    let new_uuid = uuid::Uuid::new_v4();

    let mut filepath = PathBuf::from(&CONFIG.upload_dir);
    filepath.push(&new_uuid.to_string());
    filepath.set_extension(OUTPUT_CONFIG.convert_format.unwrap().extensions_str()[0]);

    // Insert into db
    sqlx::query!(
        "insert into image (id, user_id, filepath, mime) values ($1, $2, $3, $4)",
        new_uuid,
        user_id,
        filepath.to_str(),
        OUTPUT_CONFIG.convert_format.unwrap().to_mime_type()
    )
    .execute(&state.db)
    .await.map_err(handle_internal_error)?;

    // Process and persist image
    tokio::task::spawn_blocking(move || {
        process_image(image.contents.path(), &filepath, &OUTPUT_CONFIG)
    }).await
        .map_err(handle_internal_error)?
        .map_err(handle_internal_error)?;

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
        eprintln!("Database model is missing fields: {:?}", db_model);
        return not_found_err;
    }

    let path = PathBuf::from(db_model.filepath.unwrap());

    // Ensure file exists
    if !path.is_file() {
        eprintln!("File does not exist at path: {:?}", path);
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
) -> Result<(), (StatusCode, String)> {
    let db_record = sqlx::query!("select * from image where id=$1", image_id)
        .fetch_optional(&state.db)
        .await.map_err(handle_internal_error)?;

    // TODO: User authorization check

    if db_record.is_none() {
        return Err((StatusCode::NOT_FOUND, "Image not found".to_string()));
    }

    if let Err(e) = sqlx::query!("delete from image where id=$1", image_id)
        .execute(&state.db)
        .await {
        return Err(handle_internal_error(e));
    }

    Ok(())
}
