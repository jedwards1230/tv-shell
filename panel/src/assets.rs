//! Static assets, embedded at compile time (self-contained: no filesystem
//! reads at runtime, no CDN).

use axum::http::header;

/// `GET /assets/htmx.min.js` — vendored htmx 2.0.4.
pub async fn htmx_js() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../assets/htmx.min.js"),
    )
}

/// `GET /assets/style.css` — the panel's admin stylesheet.
pub async fn style_css() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../assets/style.css"),
    )
}
