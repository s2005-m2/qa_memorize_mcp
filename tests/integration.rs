use std::sync::Arc;

use memorize_mcp::embedding::Embedder;
use memorize_mcp::server::MemorizeServer;
use memorize_mcp::storage::Storage;
use rmcp::model::*;
use rmcp::{ClientHandler, ServiceExt};

#[derive(Default, Clone)]
struct TestClient;
impl ClientHandler for TestClient {}

/// Create a real MemorizeServer backed by a temp directory.
/// Returns (server, _tempdir) — caller must hold _tempdir to keep it alive.
async fn test_server() -> (MemorizeServer, tempfile::TempDir) {
    let embedder = Arc::new(
        Embedder::load(
            "embedding_model/model_ort.onnx",
            "embedding_model/tokenizer.json",
        )
        .expect("Failed to load embedder"),
    );
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(
        Storage::open(dir.path().to_str().unwrap())
            .await
            .expect("Failed to open storage"),
    );
    (MemorizeServer::new(storage, embedder), dir)
}

/// Spin up an in-process MCP client+server pair over a duplex transport.
async fn setup() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, TestClient>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let (server, dir) = test_server().await;
    let (server_transport, client_transport) = tokio::io::duplex(65536);

    let server_handle = tokio::spawn(async move {
        let service = server.serve(server_transport).await.unwrap();
        let _ = service.waiting().await;
    });

    let client = TestClient
        .serve(client_transport)
        .await
        .expect("client handshake failed");

    (client, server_handle, dir)
}

fn get_text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("expected text content")
}

// ── Test 1: Store and query a QA pair ──

#[tokio::test]
async fn test_store_and_query_qa() {
    let (client, server_handle, _dir) = setup().await;

    // Store
    let store_result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "store_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "What is Rust?",
                    "answer": "A systems programming language focused on safety and performance",
                    "topic": "Programming Languages"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    assert!(!store_result.is_error.unwrap_or(false));
    let text = get_text(&store_result);
    assert!(text.contains("stored"), "expected 'stored' in: {text}");

    // Query
    let query_result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "query_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "Tell me about Rust",
                    "context": "Programming Languages"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    assert!(!query_result.is_error.unwrap_or(false));
    let text = get_text(&query_result);
    assert!(
        text.contains("systems programming language"),
        "expected answer in query results: {text}"
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}

// ── Test 2: Topic deduplication ──

#[tokio::test]
async fn test_topic_deduplication() {
    let (client, server_handle, _dir) = setup().await;

    // Store first QA under "Rust编程"
    let r1 = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "store_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "What is ownership in Rust?",
                    "answer": "A memory management system without garbage collection",
                    "topic": "Rust编程"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    let text1 = get_text(&r1);
    let v1: serde_json::Value = serde_json::from_str(text1).unwrap();
    let topic1 = v1["topic"].as_str().unwrap();

    // Store second QA under a semantically similar topic "Rust开发"
    let r2 = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "store_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "How does borrowing work in Rust?",
                    "answer": "References allow temporary access without taking ownership",
                    "topic": "Rust开发"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    let text2 = get_text(&r2);
    let v2: serde_json::Value = serde_json::from_str(text2).unwrap();
    let topic2 = v2["topic"].as_str().unwrap();

    // Both should resolve to the same topic (the first one created)
    assert_eq!(
        topic1, topic2,
        "Expected topic deduplication: '{topic1}' vs '{topic2}'"
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}

// ── Test 3: Query with no matching topic ──

#[tokio::test]
async fn test_query_no_matching_topic() {
    let (client, server_handle, _dir) = setup().await;

    // Store a QA under a specific topic
    client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "store_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "What is photosynthesis?",
                    "answer": "The process by which plants convert light to energy",
                    "topic": "Biology"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    // Query with a completely unrelated context
    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "query_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "How to cook pasta?",
                    "context": "Italian Cuisine and Cooking Recipes"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    assert!(!result.is_error.unwrap_or(false));
    let text = get_text(&result);
    assert!(
        text.contains("No matching topic found"),
        "expected 'No matching topic found' in: {text}"
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}

// ── Test 4: Multiple QA pairs under the same topic ──

#[tokio::test]
async fn test_multiple_qa_same_topic() {
    let (client, server_handle, _dir) = setup().await;

    let pairs = [
        ("What is a variable in Python?", "A named reference to a value"),
        ("What is a list in Python?", "An ordered mutable collection"),
        (
            "What is a dictionary in Python?",
            "A key-value mapping data structure",
        ),
    ];

    for (q, a) in &pairs {
        let r = client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "store_qa".into(),
                arguments: Some(
                    serde_json::json!({
                        "question": q,
                        "answer": a,
                        "topic": "Python Basics"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                task: None,
            })
            .await
            .unwrap();
        assert!(!r.is_error.unwrap_or(false));
    }

    // Query for Python data structures
    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "query_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "Python data structures",
                    "context": "Python Basics"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    assert!(!result.is_error.unwrap_or(false));
    let text = get_text(&result);
    let records: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
    assert!(
        records.len() >= 2,
        "expected at least 2 results, got {}",
        records.len()
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}

// ── Test 5: Resource template is listed ──

#[tokio::test]
async fn test_resource_template_listed() {
    let (client, server_handle, _dir) = setup().await;

    let result = client.list_resource_templates(None).await.unwrap();
    assert!(
        !result.resource_templates.is_empty(),
        "expected at least one resource template"
    );

    let tmpl = &result.resource_templates[0];
    assert!(
        tmpl.raw.uri_template.contains("knowledge://"),
        "expected knowledge:// URI template, got: {}",
        tmpl.raw.uri_template
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}

// ── Test 6: Cold start query on empty database ──

#[tokio::test]
async fn test_cold_start_query() {
    let (client, server_handle, _dir) = setup().await;

    // Query on a completely empty database
    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "query_qa".into(),
            arguments: Some(
                serde_json::json!({
                    "question": "Anything at all",
                    "context": "General Knowledge"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            task: None,
        })
        .await
        .unwrap();

    assert!(!result.is_error.unwrap_or(false));
    let text = get_text(&result);
    assert!(
        text.contains("No matching topic found"),
        "expected graceful empty response, got: {text}"
    );

    client.cancel().await.unwrap();
    let _ = server_handle.await;
}
