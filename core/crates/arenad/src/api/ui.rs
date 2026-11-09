//! Serving the built UI next to the API.

use std::path::{Component, Path as FsPath, PathBuf};

use axum::{
    http::{header, Method, StatusCode, Uri},
    response::IntoResponse,
    Router,
};

use super::ApiError;

fn content_type(p: &FsPath) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serves `dist` for GET/HEAD requests the API doesn't claim; unknown non-asset paths get
/// `index.html` (SPA). `/api/`, `/mcp/`, `/gateway/` and missing `/assets/` files are 404s.
pub fn with_ui(router: Router, dist: PathBuf) -> Router {
    router.fallback(move |method: Method, uri: Uri| {
        let dist = dist.clone();
        async move {
            let path = uri.path();
            let reserved = ["/api/", "/mcp/", "/gateway/"]
                .iter()
                .any(|p| path.starts_with(p) || path == p.trim_end_matches('/'));
            if reserved || !(method == Method::GET || method == Method::HEAD) {
                return ApiError::not_found("no such route").into_response();
            }
            let rel = path.trim_start_matches('/');
            let safe = FsPath::new(rel)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
            let candidate = dist.join(rel);
            let file = if safe && !rel.is_empty() && candidate.is_file() {
                candidate
            } else if rel.starts_with("assets/") || !safe && rel.contains("assets/") {
                return ApiError::not_found("no such file").into_response();
            } else {
                dist.join("index.html")
            };
            match tokio::fs::read(&file).await {
                Ok(bytes) => ([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, String, String) {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ct = r
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let st = r.status();
        let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (st, ct, String::from_utf8_lossy(&body).to_string())
    }

    /// A scratch folder with a built UI (`dist/`) and a secret beside it; returns the folder.
    fn ui_folder() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("arena_ui_{}", uuid::Uuid::new_v4().simple()));
        let dist = root.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), "INDEX").unwrap();
        std::fs::write(dist.join("assets/a.js"), "JS").unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        root
    }

    /// Paths that climb out of the UI folder never serve the secret.
    async fn assert_no_path_escape(app: &Router) {
        for p in [
            "/../secret.txt",
            "/%2e%2e/secret.txt",
            "/assets/../../secret.txt",
            "/%2e%2e%2fsecret.txt",
        ] {
            let (_, _, body) = call(app, "GET", p).await;
            assert!(!body.contains("SECRET"), "{p} escaped");
        }
    }

    #[tokio::test]
    async fn spa_fallback_is_scoped() {
        let root = ui_folder();
        let app = with_ui(Router::new(), root.join("dist"));

        assert_eq!(call(&app, "GET", "/").await.2, "INDEX");
        assert_eq!(call(&app, "GET", "/runs/x").await.2, "INDEX");
        let (st, ct, body) = call(&app, "GET", "/assets/a.js").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "JS"));
        assert!(ct.starts_with("text/javascript"));
        assert_eq!(
            call(&app, "GET", "/assets/missing.js").await.0,
            StatusCode::NOT_FOUND
        );
        let (st, ct, body) = call(&app, "POST", "/mcp/nope").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(
            ct.starts_with("application/json") && body.contains("error"),
            "{ct} {body}"
        );
        assert_eq!(
            call(&app, "GET", "/gateway/x").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(call(&app, "GET", "/api/x").await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "POST", "/runs/x").await.0, StatusCode::NOT_FOUND);
        assert_no_path_escape(&app).await;
        let _ = std::fs::remove_dir_all(root);
    }
}
