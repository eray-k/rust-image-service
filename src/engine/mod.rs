use axum::{extract::State, http::StatusCode, response::IntoResponse};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::{AppState, config::CONFIG, engine::validator::ValidateExt};

mod validator;

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
        mut image,
    }): TypedMultipart<UploadFileRequest>,
) -> impl IntoResponse {
    // Validate the file
    let validated_imagetype = image.contents.validate().await.unwrap();

    let new_uuid = uuid::Uuid::new_v4();

    let filepath = format!(
        "{}/{}.{}",
        CONFIG.upload_dir,
        &new_uuid,
        validated_imagetype.extension()
    );

    if let Err(e) = sqlx::query!(
        "insert into image (id, user_id, filepath, mime) values ($1, $2, $3, $4)",
        new_uuid,
        user_id,
        filepath,
        validated_imagetype.to_mime()
    )
    .execute(&state.db)
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e));
    }

    if let Err(e) = tokio::task::spawn_blocking(move || {
        let fp = &filepath;
        image.contents.persist(fp).unwrap(); // TODO: handle upload errors
    })
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e));
    }

    (StatusCode::CREATED, new_uuid.to_string())
}

pub async fn download() {
    todo!()
}
