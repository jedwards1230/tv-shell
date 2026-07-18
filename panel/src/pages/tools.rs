//! `/tools` — stub (full implementation lands in M3).

use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::SharedState;

pub async fn page(State(_state): State<SharedState>) -> impl IntoResponse {
    super::render_stub("tools", "Tools", "M3")
}
