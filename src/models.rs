use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Tool Parameters ──
// 工具参数，/// doc comment 会生成 JSON Schema description 发给 AI，// 注释仅供人类阅读

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StoreQaParams {
    // 要存储的具体问题。使用清晰独立的文本，会被向量化用于语义搜索。
    /// The specific question or problem statement to store.
    /// Use clear, self-contained text — this is vectorized for semantic search.
    /// Example: "How does Rust's borrow checker prevent data races?"
    pub question: String,
    // 问题对应的已验证答案。可包含代码片段、结构化数据或分步说明。支持 Markdown。
    /// The verified answer or solution corresponding to the question.
    /// Include code snippets, structured data, or step-by-step explanations as needed.
    /// Markdown formatting is preserved. Do NOT store speculative or unverified content.
    pub answer: String,
    // 宽泛可复用的主题名（如 "Rust Programming"）。若已有语义相似主题（≥0.80），服务器自动复用。
    /// A broad, reusable topic name for categorization (e.g. "Rust Programming", "Docker Networking").
    /// If a semantically similar topic already exists (similarity >= 0.80), the server will
    /// automatically reuse the existing topic name — the returned topic field shows the resolved name.
    /// Avoid overly specific names like "Rust Ownership Question 3".
    pub topic: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryQaParams {
    // 要在长期记忆中搜索的问题。用自然语言提问以获得最佳语义匹配。
    /// The question to search for in long-term memory.
    /// Phrased as a natural language question for best semantic matching.
    /// Example: "How does Rust manage memory without garbage collection?"
    pub question: String,
    // 描述知识领域的简短短语，用于定位正确主题。不是完整对话历史，只是领域上下文。
    /// A short phrase describing the knowledge domain, used to identify the right topic.
    /// This is NOT the full conversation history — just the domain context.
    /// Example: "Rust programming" or "system design" or "project deployment".
    /// If unsure, use a broad domain name. The server matches this against stored topics.
    pub context: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MergeKnowledgeParams {
    // 要扫描的主题名。省略则扫描所有主题。指定主题可缩小范围、减少处理时间。
    /// Topic to scan for similar QA pairs. If omitted, all topics are scanned.
    /// Use a specific topic name to limit scope and reduce processing time.
    pub topic: Option<String>,
    // 合并聚类的余弦相似度阈值。范围 0.0-1.0，默认 0.85。越高越严格（合并越少）。
    /// Cosine similarity threshold for grouping QA pairs into merge clusters.
    /// Range: 0.0 to 1.0. Default: 0.85. Higher = stricter matching (fewer merges).
    /// Recommended: 0.80-0.90. Below 0.75 may merge unrelated pairs.
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaEntry {
    pub question: String,
    pub answer: String,
    pub topic: String,
    pub merged: bool,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub knowledge_text: String,
    pub topic: String,
    pub source_questions: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
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
