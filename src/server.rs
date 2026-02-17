use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::common::schema_for_type,
    model::*,
    service::{RequestContext, RoleServer},
};
use serde_json::json;

use crate::embedding::Embedder;
use crate::models::*;
use crate::storage::Storage;

// ── Server Instructions ──
// 服务器级说明书，注入 LLM system prompt。纯英文发给 AI，中文注释仅供人类阅读。

const SERVER_INSTRUCTIONS: &str = concat!(
    // 标题
    "## Memorize MCP — Tool Usage Manual\n",
    "\n",
    // 角色定义：你连接了一个基于 RAG 的长期记忆服务器
    "### Role\n",
    "You are connected to a RAG-based long-term memory server that stores accumulated experience and verified knowledge. ",
    "Use it to persist verified knowledge as QA pairs, ",
    "retrieve relevant context from past interactions, ",
    "and consolidate overlapping information into refined knowledge entries.\n",
    "\n",
    "**CRITICAL — Proactive Retrieval**: When you encounter experience-dependent questions ",
    "(unfamiliar libraries, framework quirks, best practices, debugging patterns, configuration issues, ",
    "or any question where past experience would help), ALWAYS call `query_qa` FIRST — before searching ",
    "the web, reading docs, or reasoning from scratch. Past experience stored here is pre-verified and ",
    "more reliable than general search results.\n",
    "\n",
    // 工具调用原则
    "### Tool Invocation Principles\n",
    // 1. 先查后存：store 之前必须先 query 检查是否已有相似知识
    "1. **Query before Store**: Always call `query_qa` first to check if similar knowledge already exists. ",
    "Only call `store_qa` when no sufficiently relevant result is found (score > 0.80 means a match exists).\n",
    // 2. 原子化 QA：每次只存一问一答，不要把多个事实塞进同一条
    "2. **Atomic QA pairs**: Each `store_qa` call should contain exactly ONE question and ONE answer. ",
    "Do not bundle multiple facts into a single QA pair — split them.\n",
    // 3. 主题粒度一致：用宽泛可复用的主题名，服务器按语义相似度(0.80)自动去重
    "3. **Consistent topic granularity**: Use broad, reusable topic names ",
    "(e.g. \"Rust Programming\", \"Project Architecture\") ",
    "rather than overly specific ones (e.g. \"Rust Ownership Question 3\"). ",
    "The server deduplicates topics by semantic similarity (threshold 0.80), but consistent naming helps.\n",
    // 4. context 字段决定搜索哪个主题，填知识领域短语而非完整对话历史
    "4. **Context field matters**: In `query_qa`, the `context` field determines which topic to search in. ",
    "Provide a short phrase describing the knowledge domain, not the full conversation history.\n",
    // 5. 定期合并：主题积累 10+ 条 QA 时调用 merge_knowledge，需要客户端支持 sampling
    "5. **Merge periodically**: Call `merge_knowledge` when a topic accumulates many QA pairs (10+). ",
    "This consolidates redundant entries into concise knowledge summaries via LLM synthesis. ",
    "Requires sampling capability from the MCP client.\n",
    "\n",
    // 工作流
    "### Workflow\n",
    "```\n",
    // 用户提问 → 总是先检索
    "User asks a question\n",
    "       │\n",
    "       ▼\n",
    "  query_qa(question=..., context=...)  ← always try retrieval first\n",
    "       │\n",
    // score < 0.5 强匹配 → 用检索结果增强回答
    "       ├─ Results found (score < 0.5) → use retrieved QA to enhance your answer\n",
    // score ≥ 0.5 弱匹配 → 用自身知识回答
    "       ├─ Results found (score ≥ 0.5) → weak match, answer from your own knowledge\n",
    // 无结果 → 必须解决问题，然后存储经验
    "       └─ No results or no relevant match:\n",
    "              1. Resolve the question by other means (web search, docs, reasoning)\n",
    "              2. MUST call store_qa to save the resolved answer — this is NOT optional\n",
    "              (Memory grows by filling gaps. Every miss is a future hit.)\n",
    "       │\n",
    "       ▼\n",
    // 定期或用户要求时 → 合并相似 QA 对
    "  Periodically (or on user request):\n",
    "  merge_knowledge(topic=...)  → consolidates similar QA pairs\n",
    "```\n",
    "\n",
    // 资源模板：只读访问已合并的知识条目，适合被动上下文注入
    "### Resource Template\n",
    "`knowledge://{topic}/{query}` — Read-only access to merged knowledge entries. ",
    "Use this for passive context injection rather than active tool calls. ",
    "Returns up to 5 results ranked by semantic similarity.\n",
    "\n",
    // 返回格式说明
    "### Response Format\n",
    // store_qa 返回已解析的主题名，可能因语义去重而与输入不同
    "- `store_qa` returns: `{\"status\": \"stored\", \"topic\": \"<resolved_topic>\"}` ",
    "The resolved_topic may differ from your input if a semantically similar topic already existed.\n",
    // query_qa 返回 QA 数组，score 是 L2 距离（越小越相似）
    "- `query_qa` returns: array of `{\"question\", \"answer\", \"topic\", \"merged\", \"score\"}` ",
    "where score is L2 distance (lower = more similar, 0.0 = exact match, >1.0 = weak match).\n",
    // merge_knowledge 返回合并摘要
    "- `merge_knowledge` returns: merge summary with count of consolidated pairs per topic.\n",
    "\n",
    // 约束条件
    "### Constraints\n",
    // 所有文本在本地向量化（384 维 ONNX），不调用外部 API
    "- All text is embedded locally (384-dim ONNX model). No external API calls for embedding.\n",
    // merge_knowledge 需要客户端支持 sampling，否则会失败
    "- `merge_knowledge` requires the MCP client to support sampling (createMessage). ",
    "It will fail if sampling is unavailable.\n",
    // 合并时每个主题最多扫描 100 条 QA，主题很大时需多次运行
    "- Maximum 100 QA pairs scanned per topic during merge. For very large topics, run merge multiple times.\n",
);

// ── Tool Descriptions ──
// 工具描述，出现在 tools/list 响应中

// 持久化已验证的 QA 对到长期记忆。主题按语义自动去重(≥0.80)。
// 重要：存之前先调 query_qa 检查。只存已验证的事实，不存推测性内容。
const STORE_QA_DESC: &str = "\
Persist a verified question-answer pair to long-term memory under a semantic topic. \
Topics are automatically deduplicated: if a semantically similar topic already exists (cosine similarity ≥ 0.80), \
the existing topic name is reused instead of creating a duplicate. \
IMPORTANT: Call query_qa first to check for existing knowledge before storing. \
Only store facts that have been verified or confirmed — do not store speculative or uncertain information.";

// 两阶段语义搜索：先用 context 定位主题，再用 question 在主题内搜索。
// 返回最多 5 条结果，score 是 L2 距离（0.0=精确匹配，>1.0=弱匹配）。
// 应在 store_qa 之前调用以避免重复，也用于为用户问题检索上下文。
const QUERY_QA_DESC: &str = "\
Search long-term memory for relevant QA pairs using semantic similarity. \
WHEN TO USE: Proactively call this tool when facing experience-dependent questions — \
unfamiliar libraries, framework quirks, best practices, debugging patterns, configuration issues, \
or any situation where past experience would help. Check here BEFORE searching the web or docs. \
The search is two-phase: first, the `context` field is used to identify the most relevant topic; \
then, the `question` field is used to find matching QA pairs within that topic. \
Returns up to 5 results sorted by relevance. Each result includes a `score` field (L2 distance): \
0.0 = exact match, < 0.5 = strong match, 0.5–1.0 = moderate match, > 1.0 = weak/no match. \
Also use BEFORE store_qa to avoid storing duplicates.";

// 将主题内语义相似的 QA 对聚类，通过 MCP sampling 调用 LLM 合并为精炼知识条目。
// 已合并的 QA 会被标记，不再出现在 query_qa 结果中。
// 需要客户端支持 sampling。适用于主题积累 10+ 条 QA 或 query_qa 返回大量重叠结果时。
const MERGE_KNOWLEDGE_DESC: &str = "\
Consolidate similar QA pairs within a topic into refined knowledge summaries using LLM synthesis. \
Scans for QA pairs whose questions are semantically similar (above the threshold), \
groups them into clusters, and uses MCP sampling (createMessage) to merge each cluster \
into a single concise knowledge entry. Merged QA pairs are marked and excluded from future query_qa results. \
REQUIRES: The MCP client must support sampling capability. \
WHEN TO USE: When a topic has accumulated 10+ QA pairs, or when query_qa returns many overlapping results. \
Omit the `topic` parameter to scan all topics at once.";

// 按主题和查询语义检索已合并的知识条目（merge_knowledge 的产物）。
// 与 query_qa 不同，这里访问的是精炼去重后的知识，适合被动上下文注入。
const KNOWLEDGE_RESOURCE_DESC: &str = "\
Search merged knowledge entries by topic and query using semantic similarity. \
Returns up to 5 consolidated knowledge summaries ranked by relevance. \
Unlike query_qa (which searches raw QA pairs), this resource accesses the refined, \
deduplicated knowledge produced by merge_knowledge. \
Use this for passive context enrichment — the MCP client can auto-inject these results \
without an explicit tool call.";

#[derive(Clone)]
pub struct MemorizeServer {
    storage: Arc<Storage>,
    embedder: Arc<Embedder>,
}

impl MemorizeServer {
    pub fn new(storage: Arc<Storage>, embedder: Arc<Embedder>) -> Self {
        Self { storage, embedder }
    }

    // ── Tool: store_qa ──

    async fn handle_store_qa(&self, params: StoreQaParams) -> Result<CallToolResult, ErrorData> {
        let topic_vec = self.embedder.embed(&params.topic).map_err(internal)?;

        let resolved_topic = match self
            .storage
            .find_similar_topic(&topic_vec, DEFAULT_TOPIC_THRESHOLD)
            .await
            .map_err(internal)?
        {
            Some(existing) => existing,
            None => {
                self.storage
                    .create_topic(&params.topic, &topic_vec)
                    .await
                    .map_err(internal)?;
                params.topic.clone()
            }
        };

        let q_vec = self.embedder.embed(&params.question).map_err(internal)?;
        self.storage
            .insert_qa(&params.question, &params.answer, &resolved_topic, &q_vec)
            .await
            .map_err(internal)?;

        Ok(CallToolResult::success(vec![Content::text(
            json!({ "status": "stored", "topic": resolved_topic }).to_string(),
        )]))
    }

    // ── Tool: query_qa ──

    async fn handle_query_qa(&self, params: QueryQaParams) -> Result<CallToolResult, ErrorData> {
        let ctx_vec = self.embedder.embed(&params.context).map_err(internal)?;

        let topic = match self
            .storage
            .find_similar_topic(&ctx_vec, DEFAULT_TOPIC_THRESHOLD)
            .await
            .map_err(internal)?
        {
            Some(t) => t,
            None => {
                return Ok(CallToolResult::success(vec![Content::text(
                    json!({ "message": "No matching topic found", "results": [] }).to_string(),
                )]));
            }
        };

        let q_vec = self.embedder.embed(&params.question).map_err(internal)?;
        let results = self
            .storage
            .search_qa(&q_vec, &topic, DEFAULT_SEARCH_LIMIT)
            .await
            .map_err(internal)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&results).map_err(internal)?,
        )]))
    }

    // ── Tool: merge_knowledge ──

    async fn handle_merge_knowledge(
        &self,
        params: MergeKnowledgeParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let threshold = params.threshold.unwrap_or(DEFAULT_MERGE_THRESHOLD);

        let topics = match &params.topic {
            Some(t) => vec![t.clone()],
            None => self.storage.list_topics().await.map_err(internal)?,
        };

        let mut total_merges = 0u32;
        let mut summary_parts: Vec<String> = Vec::new();

        for topic in &topics {
            // Get all unmerged QA records for this topic.
            // Use a zero vector to retrieve broadly, relying on the topic filter.
            let zero_vec = vec![0.0f32; VECTOR_DIM as usize];
            let all_qa = self
                .storage
                .search_qa(&zero_vec, topic, 100)
                .await
                .map_err(internal)?;

            if all_qa.is_empty() {
                continue;
            }

            // Track which questions have already been clustered
            let mut clustered: Vec<bool> = vec![false; all_qa.len()];

            for i in 0..all_qa.len() {
                if clustered[i] {
                    continue;
                }

                let anchor_vec = self.embedder.embed(&all_qa[i].question).map_err(internal)?;
                let similar = self
                    .storage
                    .find_similar_qa(&anchor_vec, topic, threshold)
                    .await
                    .map_err(internal)?;

                // Build cluster: mark anchor and all similar items
                clustered[i] = true;
                let mut cluster_indices: Vec<usize> = vec![i];

                for sim in &similar {
                    if let Some(idx) = all_qa.iter().position(|q| q.question == sim.question) {
                        if !clustered[idx] {
                            clustered[idx] = true;
                            cluster_indices.push(idx);
                        }
                    }
                }

                // Need at least 2 QA pairs to merge
                if cluster_indices.len() < 2 {
                    continue;
                }

                // Build merge prompt
                let mut merge_prompt =
                    String::from("Merge the following QA pairs into a concise knowledge summary:\n\n");
                for (j, &idx) in cluster_indices.iter().enumerate() {
                    merge_prompt.push_str(&format!(
                        "QA {}:\nQ: {}\nA: {}\n\n",
                        j + 1,
                        all_qa[idx].question,
                        all_qa[idx].answer
                    ));
                }

                // Use sampling to merge via LLM
                let response = context
                    .peer
                    .create_message(CreateMessageRequestParams {
                        meta: None,
                        task: None,
                        messages: vec![SamplingMessage::user_text(&merge_prompt)],
                        model_preferences: Some(ModelPreferences {
                            hints: Some(vec![ModelHint {
                                name: Some("claude".to_string()),
                            }]),
                            cost_priority: Some(0.3),
                            speed_priority: Some(0.5),
                            intelligence_priority: Some(0.8),
                        }),
                        system_prompt: Some(
                            "You are a knowledge synthesis assistant. Merge the following QA pairs \
                             into a concise, comprehensive knowledge summary. Preserve all important \
                             information but eliminate redundancy."
                                .to_string(),
                        ),
                        include_context: Some(ContextInclusion::None),
                        temperature: Some(0.3),
                        max_tokens: 2000,
                        stop_sequences: None,
                        metadata: None,
                        tools: None,
                        tool_choice: None,
                    })
                    .await
                    .map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Sampling failed: {}", e),
                            None,
                        )
                    })?;

                let merged_text = response
                    .message
                    .content
                    .first()
                    .and_then(|c| c.as_text())
                    .map(|t| t.text.clone())
                    .unwrap_or_default();

                if merged_text.is_empty() {
                    continue;
                }

                let knowledge_vec = self.embedder.embed(&merged_text).map_err(internal)?;
                let source_questions: Vec<String> = cluster_indices
                    .iter()
                    .map(|&idx| all_qa[idx].question.clone())
                    .collect();

                self.storage
                    .insert_knowledge(&merged_text, topic, &source_questions, &knowledge_vec)
                    .await
                    .map_err(internal)?;

                self.storage
                    .mark_merged(&source_questions)
                    .await
                    .map_err(internal)?;

                total_merges += 1;
                summary_parts.push(format!(
                    "Topic '{}': merged {} QA pairs",
                    topic,
                    cluster_indices.len()
                ));
            }
        }

        let summary = if total_merges == 0 {
            json!({ "status": "no_merges", "message": "No similar QA pairs found to merge" })
        } else {
            json!({
                "status": "merged",
                "total_merges": total_merges,
                "details": summary_parts
            })
        };

        Ok(CallToolResult::success(vec![Content::text(
            summary.to_string(),
        )]))
    }
}

// ── ServerHandler ──

impl ServerHandler for MemorizeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(SERVER_INSTRUCTIONS.into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "memorize-mcp".into(),
                title: None,
                version: env!("CARGO_PKG_VERSION").into(),
                description: None,
                icons: None,
                website_url: None,
            },
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: vec![
                Tool {
                    name: "store_qa".into(),
                    title: None,
                    description: Some(STORE_QA_DESC.into()),
                    input_schema: schema_for_type::<StoreQaParams>(),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                },
                Tool {
                    name: "query_qa".into(),
                    title: None,
                    description: Some(QUERY_QA_DESC.into()),
                    input_schema: schema_for_type::<QueryQaParams>(),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                },
                Tool {
                    name: "merge_knowledge".into(),
                    title: None,
                    description: Some(MERGE_KNOWLEDGE_DESC.into()),
                    input_schema: schema_for_type::<MergeKnowledgeParams>(),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                },
            ],
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        match request.name.as_ref() {
            "store_qa" => {
                let params: StoreQaParams = serde_json::from_value(
                    serde_json::Value::Object(request.arguments.unwrap_or_default()),
                )
                .map_err(|e| {
                    ErrorData::invalid_params(format!("Invalid store_qa params: {}", e), None)
                })?;
                self.handle_store_qa(params).await
            }
            "query_qa" => {
                let params: QueryQaParams = serde_json::from_value(
                    serde_json::Value::Object(request.arguments.unwrap_or_default()),
                )
                .map_err(|e| {
                    ErrorData::invalid_params(format!("Invalid query_qa params: {}", e), None)
                })?;
                self.handle_query_qa(params).await
            }
            "merge_knowledge" => {
                let params: MergeKnowledgeParams = serde_json::from_value(
                    serde_json::Value::Object(request.arguments.unwrap_or_default()),
                )
                .map_err(|e| {
                    ErrorData::invalid_params(
                        format!("Invalid merge_knowledge params: {}", e),
                        None,
                    )
                })?;
                self.handle_merge_knowledge(params, context).await
            }
            _ => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Unknown tool: {}", request.name),
                None,
            )),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![RawResourceTemplate {
                uri_template: "knowledge://{topic}/{query}".into(),
                name: "Knowledge Base".into(),
                title: Some("Knowledge Base Search".into()),
                description: Some(KNOWLEDGE_RESOURCE_DESC.into()),
                mime_type: Some("application/json".into()),
                icons: None,
            }
            .no_annotation()],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        // Parse URI: knowledge://{topic}/{query}
        let uri = &request.uri;
        let path = uri.strip_prefix("knowledge://").ok_or_else(|| {
            ErrorData::resource_not_found(
                format!("Invalid knowledge URI: {}", uri),
                None,
            )
        })?;

        let (topic, query) = path.split_once('/').ok_or_else(|| {
            ErrorData::resource_not_found(
                format!(
                    "URI must have format knowledge://{{topic}}/{{query}}, got: {}",
                    uri
                ),
                None,
            )
        })?;

        if topic.is_empty() || query.is_empty() {
            return Err(ErrorData::resource_not_found(
                "Topic and query must not be empty",
                None,
            ));
        }

        let query_vec = self.embedder.embed(query).map_err(internal)?;
        let results = self
            .storage
            .search_knowledge(&query_vec, topic, DEFAULT_SEARCH_LIMIT)
            .await
            .map_err(internal)?;

        let text = serde_json::to_string_pretty(&results).map_err(internal)?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContents::text(text, uri.clone())],
        })
    }
}

// ── Helpers ──

fn internal(e: impl std::fmt::Display) -> ErrorData {
    tracing::error!("Internal error: {}", e);
    ErrorData::new(ErrorCode::INTERNAL_ERROR, format!("{}", e), None)
}
