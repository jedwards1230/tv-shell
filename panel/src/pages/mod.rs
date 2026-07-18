//! Page handlers. `dashboard`, `logs`, `dev`, `settings`, `widgets`, `tools`,
//! and `processes` are fully implemented; `controllers` and `cec` render an
//! honest stub via [`render_stub`] until M4 lands.

pub mod cec;
pub mod controllers;
pub mod dashboard;
pub mod dev;
pub mod logs;
pub mod processes;
pub mod settings;
pub mod tools;
pub mod widgets;

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

#[derive(Template)]
#[template(path = "stub.html")]
struct StubTemplate {
    active: &'static str,
    title: &'static str,
    milestone: &'static str,
}

/// Render the shared "coming in a later milestone" stub for a not-yet-built
/// page.
pub fn render_stub(
    active: &'static str,
    title: &'static str,
    milestone: &'static str,
) -> Html<String> {
    let tmpl = StubTemplate {
        active,
        title,
        milestone,
    };
    Html(
        tmpl.render()
            .unwrap_or_else(|e| format!("<p>template error: {e}</p>")),
    )
}

/// Render any askama template to a 200 HTML response, or a 500 plain-text
/// response on a render error.
pub fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {e}"),
        )
            .into_response(),
    }
}
