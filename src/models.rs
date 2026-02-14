use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Tool Parameters ──

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StoreQaParams {
    /// The question text
    pub question: String,
    /// The answer text  
    pub answer: String,
    /// Topic name (will be deduplicated against existing topics)
    pub topic: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryQaParams {
    /// The question to search for
    pub question: String,
    /// Background context for topic matching
    pub context: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MergeKnowledgeParams {
    /// Topic to scan (optional, scans all if omitted)
    pub topic: Option<String>,
    /// Similarity threshold for merging (default: 0.85)
    pub threshold: Option<f32>,
}

// ── Data Records ──

#[derive(Debug, Clone, Serialize)]
pub struct QaRecord {
    pub question: String,
    pub answer: String,
    pub topic: String,
    pub merged: bool,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeRecord {
    pub knowledge_text: String,
    pub topic: String,
    pub source_questions: Vec<String>,
    pub score: f32,
}

// ── Constants ──
pub const VECTOR_DIM: i32 = 384;
pub const DEFAULT_MERGE_THRESHOLD: f32 = 0.85;
pub const DEFAULT_TOPIC_THRESHOLD: f32 = 0.80;
pub const DEFAULT_SEARCH_LIMIT: usize = 5;
