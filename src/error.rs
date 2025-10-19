use axum::http::StatusCode;

pub fn handle_internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    eprintln!("Internal server error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}
