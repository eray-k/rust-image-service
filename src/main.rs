use axum::{extract::{DefaultBodyLimit, State}, http::StatusCode, routing::post, Router};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use sqlx::PgPool;
use tempfile::NamedTempFile;
use tokio::fs;
use uuid::Uuid;

use crate::config::CONFIG;

mod config;

enum ImageType {
    Jpeg,
    Png,
    Webp
}

impl ImageType {
    fn from_mime(mime: &str) -> Option<ImageType> {
        match mime {
            "image/jpeg" => Some(ImageType::Jpeg),
            "image/png" => Some(ImageType::Png),
            "image/webp" => Some(ImageType::Webp),
            _ => None
        }
    }

    fn extension(&self) -> &'static str {
        match self {
            ImageType::Jpeg => "jpg",
            ImageType::Png => "png",
            ImageType::Webp => "webp"
        }
    }
}

#[derive(TryFromMultipart)]
struct UploadFileRequest {
    user_id: Uuid,
    #[form_data(limit = "2MiB")]
    image: FieldData<NamedTempFile>,
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

async fn upload(
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
        Ok(_) => (StatusCode::OK, "done"),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Error")  // TODO: Revert the process, delete the file
    }
    
}

#[derive(Clone)]
struct AppState {
    db: PgPool
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the database
    let db = sqlx::postgres::PgPool::connect(&CONFIG.db_url).await.expect("Couldn't connect to database");

    sqlx::migrate!("./migrations").run(&db).await.expect("Migrations failed");
    
    // Prepare the upload directory
    fs::create_dir_all(&CONFIG.upload_dir).await.expect("Couldn't create upload directory");
    
    let app_state = AppState {db};

    let app = Router::new()
        .route("/", post(upload))
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(1024 * 1024 * 5)); // 5 MB

    
    let addr = "0.0.0.0:7070".to_string();
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Couldn't bind to tcp listener");

    axum::serve(listener, app).await.unwrap();
    Ok(())
}
