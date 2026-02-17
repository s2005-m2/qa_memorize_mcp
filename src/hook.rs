use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};

use crate::embedding::Embedder;
use crate::models::{DEFAULT_SEARCH_LIMIT, DEFAULT_TOPIC_THRESHOLD};
use crate::storage::Storage;

#[derive(Clone)]
struct AppState {
    storage: Arc<Storage>,
    embedder: Arc<Embedder>,
}

#[derive(Deserialize)]
struct RecallParams {
    q: Option<String>,
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
    let q = match params.q.filter(|s| !s.is_empty()) {
        Some(q) => q,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!([]))),
    };
    let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);

    let q_vec = match state.embedder.embed(&q) {
        Ok(v) => v,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!([]))),
    };

    let mut items: Vec<RecallItem> = Vec::new();

    if let Some(ctx) = params.context.filter(|s| !s.is_empty()) {
        let ctx_vec = match state.embedder.embed(&ctx) {
            Ok(v) => v,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!([]))),
        };
        if let Ok(Some(topic)) = state.storage.find_similar_topic(&ctx_vec, DEFAULT_TOPIC_THRESHOLD).await {
            if let Ok(qas) = state.storage.search_qa(&q_vec, &topic, limit).await {
                for r in qas {
                    items.push(RecallItem {
                        kind: "qa",
                        question: Some(r.question),
                        answer: Some(r.answer),
                        text: None,
                        topic: r.topic,
                        score: r.score,
                    });
                }
            }
            if let Ok(kns) = state.storage.search_knowledge(&q_vec, &topic, limit).await {
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
    } else {
        if let Ok(qas) = state.storage.find_nearest_qa_global_n(&q_vec, limit).await {
            for r in qas {
                items.push(RecallItem {
                    kind: "qa",
                    question: Some(r.question),
                    answer: Some(r.answer),
                    text: None,
                    topic: r.topic,
                    score: r.score,
                });
            }
        }
        if let Ok(kns) = state.storage.find_nearest_knowledge_global_n(&q_vec, limit).await {
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
