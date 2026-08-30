use axum::{
    body::Body,
    http::{Response, StatusCode, Uri, header},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct UiAssets;

pub(super) async fn asset(uri: Uri) -> Response<Body> {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    if let Some(asset) = UiAssets::get(path) {
        return response(path, asset.data.into_owned(), StatusCode::OK);
    }
    if !path.starts_with("api/")
        && !path.starts_with("debug/")
        && !path.starts_with("diagnostics/")
        && !path.contains('.')
        && let Some(index) = UiAssets::get("index.html")
    {
        return response("index.html", index.data.into_owned(), StatusCode::OK);
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("not found"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn response(path: &str, bytes: Vec<u8>, status: StatusCode) -> Response<Body> {
    let cache_control = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(path).first_or_octet_stream().as_ref(),
        )
        .header(header::CACHE_CONTROL, cache_control)
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "same-origin")
        .header(
            "content-security-policy",
            "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'self'; \
            connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'",
        )
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
