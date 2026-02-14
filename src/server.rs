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
            instructions: Some(
                "RAG-based memory server for persistent QA storage and knowledge fusion".into(),
            ),
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
                    description: Some(
                        "Store a question-answer pair under a topic. Topics are deduplicated by semantic similarity.".into(),
                    ),
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
                    description: Some(
                        "Search for relevant QA pairs by question, using context to find the right topic.".into(),
                    ),
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
                    description: Some(
                        "Merge similar QA pairs into consolidated knowledge entries using LLM sampling.".into(),
                    ),
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
                description: Some(
                    "Search merged knowledge entries by topic and query".into(),
                ),
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
