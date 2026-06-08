//! Static asset server. Serves files from the embedded `web/build/`
//! directory; falls back to `index.html` for SPA routes.

use super::assets::WebAssets;
use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};

pub async fn serve_static(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(bytes) = WebAssets::get(path) {
        return file_response(path, bytes);
    }
    let with_slash = format!("/{}", path);
    if let Some(bytes) = WebAssets::get(&with_slash) {
        return file_response(path, bytes);
    }
    if let Some(bytes) = WebAssets::get("index.html") {
        return file_response("index.html", bytes);
    }

    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain")],
        "404 — web bundle not built. Run `pnpm build` in web/ first.",
    )
        .into_response()
}

fn file_response(path: &str, bytes: &'static [u8]) -> Response {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes.to_vec()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
