use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};

use crate::embedding::Embedder;
use crate::models::DEFAULT_SEARCH_LIMIT;
use crate::storage::Storage;

/// Looser threshold for recall: user prompts are full sentences,
/// not topic names, so they need more room to match.
const RECALL_TOPIC_THRESHOLD: f32 = 0.60;

#[derive(Clone)]
struct AppState {
    storage: Arc<Storage>,
    embedder: Arc<Embedder>,
}

#[derive(Deserialize)]
struct RecallParams {
    context: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct RecallItem {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    topic: String,
    score: f32,
}

async fn recall_handler(
    State(state): State<AppState>,
    Query(params): Query<RecallParams>,
) -> impl IntoResponse {
    let ctx = match params.context.filter(|s| !s.is_empty()) {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!([]))),
    };
    let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);

    let ctx_vec = match state.embedder.embed(&ctx) {
        Ok(v) => v,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!([]))),
    };

    let mut items: Vec<RecallItem> = Vec::new();

    if let Ok(Some(topic)) = state.storage.find_similar_topic(&ctx_vec, RECALL_TOPIC_THRESHOLD).await {
        if let Ok(kns) = state.storage.search_knowledge(&ctx_vec, &topic, limit).await {
            for r in kns {
                items.push(RecallItem {
                    kind: "knowledge",
                    question: None,
                    answer: None,
                    text: Some(r.knowledge_text),
                    topic: r.topic,
                    score: r.score,
                });
            }
        }
    }

    items.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
    (StatusCode::OK, Json(serde_json::json!(items)))
}

pub fn recall_router(storage: Arc<Storage>, embedder: Arc<Embedder>) -> Router {
    let state = AppState { storage, embedder };
    Router::new()
        .route("/api/recall", get(recall_handler))
        .with_state(state)
}
