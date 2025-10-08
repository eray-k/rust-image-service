use axum::{extract::State, http::StatusCode};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::{config::CONFIG, image_type::ImageType, AppState};

#[derive(TryFromMultipart)]
pub struct UploadFileRequest {
    user_id: Uuid,
    #[form_data(limit = "2MiB")]
    image: FieldData<NamedTempFile>
}

async fn validate_file(image: &FieldData<NamedTempFile>) -> Result<ImageType, Box<dyn std::error::Error>> {
    // TODO: validate with "magic bytes"

    let mime = &image.metadata.content_type;

    if let Some(image_type) = ImageType::from_mime(mime.as_deref().unwrap_or("")) {
        Ok(image_type)
    } else {
        Err("Invalid file type".into())
    }
}

pub async fn upload(
    state: State<AppState>,
    TypedMultipart(UploadFileRequest {user_id, image}) : TypedMultipart<UploadFileRequest>) 
    -> (StatusCode, &'static str) {
    // Validate the file
    let image_type = match validate_file(&image).await {
        Ok(t) => t,
        Err(_) => return (StatusCode::BAD_REQUEST, "couldnt validate file!")
    };

    let new_uuid = uuid::Uuid::new_v4();
    let filepath = format!("{}/{}.{}", CONFIG.upload_dir, &new_uuid, image_type.extension());

    image.contents.persist(&filepath).unwrap(); // TODO: handle upload errors

    match sqlx::query!("insert into image (id, user_id, filepath) values ($1, $2, $3)", new_uuid, user_id, filepath)
    .execute(&state.db).await {
        Ok(_) => (StatusCode::CREATED, "done"),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Error")  // TODO: Revert the process, delete the file
    }
    
}

