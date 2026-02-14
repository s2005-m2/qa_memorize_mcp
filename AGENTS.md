# AGENTS.md — Memorize MCP

## Build & Run

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo check                    # type-check only (fast)
```

## Test Commands

All tests require `--test-threads=1` (LanceDB uses temp dirs that need isolation).
Tests also require the ONNX model at `embedding_model/model_ort.onnx` and `embedding_model/tokenizer.json`.
ONNX Runtime >= 1.23 must be available (via `pip install onnxruntime` or `ORT_DYLIB_PATH`).

```bash
# All tests (unit + integration + doc)
cargo test -- --test-threads=1

# Unit tests only (embedding + storage modules)
cargo test --lib -- --test-threads=1

# Integration tests only (end-to-end MCP protocol over duplex transport)
cargo test --test integration -- --test-threads=1

# Single test by name
cargo test --lib embedding::tests::test_embed_basic -- --test-threads=1
cargo test --test integration test_store_and_query_qa -- --test-threads=1
```

No rustfmt.toml or clippy.toml — use default `cargo fmt` and `cargo clippy`.

## Project Layout

```
src/lib.rs          — pub mod re-exports (embedding, models, server, storage)
src/main.rs         — CLI arg parsing + transport setup (stdio / HTTP). Binary entrypoint.
src/server.rs       — MemorizeServer: MCP ServerHandler with 3 tools + 1 resource template
src/embedding.rs    — Embedder: ONNX Runtime inference (text → 384-dim f32 vector)
src/storage.rs      — Storage: LanceDB CRUD for 3 tables (topics, qa_records, knowledge)
src/models.rs       — Shared data structs (tool params, records) + constants
tests/integration.rs — End-to-end tests using real Embedder + Storage over in-process MCP
scripts/package.py  — Cross-platform packaging (downloads ONNX Runtime, assembles dist/)
```

## Code Style

### Imports

Group imports in this order, separated by blank lines:
1. `std::` imports
2. External crate imports (one `use` block per crate, alphabetical)
3. `crate::` / local imports

```rust
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rmcp::{ServerHandler, model::*, service::{RequestContext, RoleServer}};
use serde_json::json;

use crate::embedding::Embedder;
use crate::models::*;
```

Nested imports are used freely: `use rmcp::{ServerHandler, model::*}`.
Glob imports (`*`) are used for `crate::models::*` and `rmcp::model::*` only.

### Error Handling

- All fallible functions return `anyhow::Result<T>` (library/internal code).
- MCP tool handlers return `Result<CallToolResult, ErrorData>` (rmcp protocol errors).
- Convert between the two with a helper: `fn internal(e: impl Display) -> ErrorData`.
- Use `.map_err(|e| anyhow!("context: {}", e))?` for ort/tokenizer errors that don't impl std Error.
- Use `.map_err(internal)?` to bridge anyhow → ErrorData in server tool handlers.
- Never use `.unwrap()` in non-test code. Tests may use `.unwrap()` and `.expect()`.

### Naming

- Structs: `PascalCase` — `MemorizeServer`, `StoreQaParams`, `QaRecord`
- Functions/methods: `snake_case` — `find_similar_topic`, `handle_store_qa`
- Constants: `SCREAMING_SNAKE_CASE` — `VECTOR_DIM`, `DEFAULT_MERGE_THRESHOLD`
- Private tool handlers: `handle_<tool_name>` methods on the server struct
- Schema builders: `<table>_schema()` free functions in storage.rs
- Test helpers: `test_server()`, `setup()`, `fake_vector()`, `get_text()`

### Types & Patterns

- Shared state via `Arc<T>`: `Arc<Storage>`, `Arc<Embedder>` on `MemorizeServer`
- `Embedder` wraps `Session` in `Mutex<Session>` because ort v2 `Session::run` needs `&mut self`
- `Storage` methods are all `&self` + async — LanceDB handles internal concurrency
- Tool param structs derive `Deserialize, JsonSchema` (schemars v1.0, not v0.8)
- Record structs derive `Serialize` for JSON responses
- Section comments use `// ── Section Name ──` style

### Async

- Runtime: tokio with `features = ["full"]`
- All Storage methods are `async fn`
- Embedder methods are sync (`fn`, not `async fn`) — ONNX inference is CPU-bound
- Integration tests use `#[tokio::test]`, unit storage tests use `#[tokio::test]`
- Unit embedding tests use plain `#[test]` (sync)

### MCP Server (rmcp v0.15)

- Manual `impl ServerHandler` — no `#[tool_router]` / `#[tool_handler]` macros
- Tool input schemas generated via `schema_for_type::<ParamStruct>()`
- Tool arguments deserialized: `serde_json::from_value(Value::Object(args.unwrap_or_default()))`
- Success: `CallToolResult::success(vec![Content::text(json_string)])`
- Errors: `ErrorData::new(ErrorCode::INTERNAL_ERROR, msg, None)` or `ErrorData::invalid_params()`
- Sampling: `context.peer.create_message(CreateMessageRequestParams { ... })`
- Resources: `ResourceContents::text(content, uri)`, templates via `RawResourceTemplate`

### LanceDB (v0.26) + Arrow (v57)

- Schemas built with `Arc<Schema>` from `arrow_schema`
- Vector columns: `FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>`
- List<Utf8> columns: `ListBuilder::new(StringBuilder::new())`
- Insert: `RecordBatch::try_new(schema, columns)` → `table.add(Box::new(batches)).execute().await`
- Query: `table.query().nearest_to(vec)?.only_if(filter).limit(n).execute().await`
- Update: `table.update().only_if(filter).column("col", "sql_expr").execute().await`
- Parse results by downcasting: `column_by_name("x").as_any().downcast_ref::<StringArray>()`

### Testing Patterns

- Unit tests: `#[cfg(test)] mod tests` inside each module, `use super::*`
- Storage tests: `tempfile::tempdir()` for isolated LanceDB instances
- Integration tests: `tokio::io::duplex(65536)` for in-process MCP client↔server
- Test client: `struct TestClient; impl ClientHandler for TestClient {}`
- Hold `TempDir` handle in test scope — dropping it deletes the directory
- `fake_vector(seed: f32) -> Vec<f32>` generates deterministic 384-dim L2-normalized vectors

## Key Constants (models.rs)

```
VECTOR_DIM = 384              # embedding output dimensionality
DEFAULT_MERGE_THRESHOLD = 0.85
DEFAULT_TOPIC_THRESHOLD = 0.80
DEFAULT_SEARCH_LIMIT = 5
```

## Runtime Dependencies

- ONNX Runtime >= 1.23.0 (loaded dynamically, NOT statically linked)
- Search order: `ORT_DYLIB_PATH` env → next to executable → Python onnxruntime package
- Model files: `embedding_model/model_ort.onnx` + `embedding_model/tokenizer.json`
- Windows system has onnxruntime 1.17 in System32 — too old, must not be used

## Gotchas

- `schemars` must be v1.0+ (rmcp v0.15 requires it, v0.8 will not compile)
- `ort` uses `load-dynamic` feature — no static linking, avoids CRT mismatch on Windows
- All tracing output goes to stderr — stdout is reserved for stdio JSON-RPC transport
- LanceDB `_distance` column: lower = more similar. For L2 on normalized vectors: distance ≈ 1 - cosine_similarity
- LanceDB `.column("merged", "true")` — the value is a SQL expression string, not a Rust bool
