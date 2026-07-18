//! `/cec` — stub (full implementation lands in M4).

use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::SharedState;

pub async fn page(State(_state): State<SharedState>) -> impl IntoResponse {
    super::render_stub("cec", "CEC", "M4")
}
