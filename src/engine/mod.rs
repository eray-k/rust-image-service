use axum::{extract::State, http::StatusCode};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::{AppState, config::CONFIG, engine::validator::ValidatedFile};

mod validator;

#[derive(TryFromMultipart)]
pub struct UploadFileRequest {
    user_id: Uuid,
    #[form_data(limit = "2MiB")]
    image: FieldData<NamedTempFile>,
}

pub async fn upload(
    state: State<AppState>,
    TypedMultipart(UploadFileRequest { user_id, image }): TypedMultipart<UploadFileRequest>,
) -> (StatusCode, &'static str) {
    // Validate the file
    let validated_file = ValidatedFile::try_from(image.contents).unwrap();

    let new_uuid = uuid::Uuid::new_v4();
    let filepath = format!(
        "{}/{}.{}",
        CONFIG.upload_dir,
        &new_uuid,
        validated_file.filetype.extension()
    );

    validated_file.file.persist(&filepath).unwrap(); // TODO: handle upload errors

    match sqlx::query!(
        "insert into image (id, user_id, filepath) values ($1, $2, $3)",
        new_uuid,
        user_id,
        filepath
    )
    .execute(&state.db)
    .await
    {
        Ok(_) => (StatusCode::CREATED, "done"),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Error"), // TODO: Revert the process, delete the file
    }
}
