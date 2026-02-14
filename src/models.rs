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

// ── Data Records (query results, no vector) ──

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

// ── Persistent Records (JSON export/import, with vector) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicEntry {
    pub topic_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaEntry {
    pub question: String,
    pub answer: String,
    pub topic: String,
    pub merged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub knowledge_text: String,
    pub topic: String,
    pub source_questions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorizeSnapshot {
    pub version: u32,
    pub exported_at: String,
    pub topics: Vec<TopicEntry>,
    pub qa_records: Vec<QaEntry>,
    pub knowledge: Vec<KnowledgeEntry>,
}

// ── Constants ──
pub const VECTOR_DIM: i32 = 384;
pub const DEFAULT_MERGE_THRESHOLD: f32 = 0.85;
pub const DEFAULT_TOPIC_THRESHOLD: f32 = 0.80;
pub const DEFAULT_SEARCH_LIMIT: usize = 5;
