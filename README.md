# QA Memorize MCP

[中文文档](docs/README.zh-CN.md)

Why "QA Memorize"? Because that's exactly what it does: **remember the problem-solving experience of your Agent** (Question & Answer).

Every time you chat with an AI coding agent — solving a bug, figuring out a concept, working through a tricky config — that experience vanishes when the session ends. QA Memorize MCP stores these QA pairs, distills them into knowledge, and automatically recalls relevant memories in future sessions, making your Agent smarter over time.

## Features

- **Store QA Pairs** — The agent stores valuable Q&A exchanges into a local vector database, auto-categorized by topic
- **Semantic Search** — Retrieves related memories by semantic similarity, not keyword matching
- **Knowledge Distillation** — Similar QA pairs are automatically merged into refined knowledge entries
- **Auto Recall** — Hook scripts inject relevant memories into the system prompt at the start of every new conversation
- **Shareable Knowledge** — All memories export to a human-readable `memorize_data.json` for team sharing, cross-device sync, or version control
- **Fully Local & Free** — Built-in lightweight embedding model (384-dim) and embedded LanceDB vector database. All inference and storage runs on your machine — no cloud services, no API keys, your data never leaves your computer

## Client Integration

#### Claude Code

**One-click Plugin install (recommended):**

```bash
/plugin marketplace add s2005-m2/qa_memorize_mcp
/plugin install qa-memorize-mcp@s2005-m2
```

#### Gemini CLI

**One-click Extension install (recommended):**

```bash
gemini extensions install https://github.com/s2005-m2/qa_memorize_mcp
```


#### OpenCode

**Manual configuration:**

Edit `opencode.json` (project root) or `~/.config/opencode/opencode.json` (user-level):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["npx", "-y", "qa-memorize-mcp", "--hook-port", "19533"],
      "enabled": true
    }
  }
}
```
```bash
# Download plugin file
mkdir -p .opencode/plugins
curl -o .opencode/plugins/opencode-plugin.mjs https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/hooks/opencode-plugin.mjs
```

Then add the `plugin` field to the same `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["npx", "-y", "qa-memorize-mcp", "--hook-port", "19533"],
      "enabled": true
    }
  },
  "plugin": [
    "file://./.opencode/plugins/opencode-plugin.mjs"
  ]
}
```
**Agent-oriented setup:**
```agent
Use `fetch` or `curl` to retrieve the contents of https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/docs/opencode-agent-setup.md, read it, and follow the steps to complete the setup for the user.
```

## Download MCP

```bash
npx qa-memorize-mcp
```

One command to start. Automatically downloads platform-native binary + ONNX Runtime + embedding model. No Rust toolchain required.

## MCP Tools

### store_qa

Store a QA pair. Topics are automatically deduplicated by semantic similarity.

```json
{ "question": "What is Rust's ownership?", "answer": "Rust manages memory at compile time through the ownership system.", "topic": "Rust Programming" }
```

### query_qa

Semantic search for related QA pairs. Matches topics by context first, then searches within the topic.

```json
{ "question": "How does Rust manage memory?", "context": "Programming Languages" }
```

### merge_knowledge

Scans similar QA pairs and uses MCP Sampling to request LLM to merge them into refined knowledge entries.

```json
{ "topic": "Rust Programming", "threshold": 0.85 }
```

### knowledge://{topic}/{query}

Resource template. Retrieves merged knowledge entries by topic and semantic query.

## Auto Recall API

### GET /api/recall

HTTP endpoint called by hook scripts. Matches topics by semantic similarity to the user's question and retrieves merged knowledge entries for injection into the system prompt. Requires `--hook-port` to enable.

**Query Parameters:**

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| `context` | Yes | — | User's original question text, used to match topics and search knowledge |
| `limit` | No | `5` | Maximum number of results |

**Response:** JSON array of Knowledge entries:

```json
[
  {
    "type": "knowledge",
    "text": "Rust 的所有权系统通过编译期检查...",
    "topic": "Rust 编程",
    "score": 0.31
  }
]
```

`score` is L2 distance (lower = more similar). Returns empty array `[]` when no matches.

**Recall Flow:**

```
User submits question
    │
    ▼
Client Hook triggers
    ├─ Claude Code: UserPromptSubmit event → memorize-hook.mjs
    ├─ Gemini CLI:  BeforeAgent event     → memorize-hook.mjs
    └─ OpenCode:    system.transform hook → opencode-plugin.mjs
    │
    │  GET http://localhost:19533/api/recall?context=<question>&limit=5
    │  2s timeout, silent failure (never blocks user interaction)
    │
    ▼
Hook Server (hook.rs)
    Vectorize context → match most similar topic → search Knowledge within topic
    │
    ▼
Hook script formats results → inject into system prompt
    "[Memory Recall]\nKnowledge: ..."
```

## CLI Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--transport` | `stdio` | Transport mode: `stdio` or `http` |
| `--port` | `19532` | HTTP mode listen port |
| `--hook-port` | _(disabled)_ | Hook HTTP Server port (enables `/api/recall`) |
| `--db-path` | `~/.memorize-mcp` | LanceDB database path and JSON snapshot directory |
| `--model-dir` | `./embedding_model` | Embedding model directory |
| `--debug` | _(disabled)_ | Output debug logs to stderr |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        main.rs                           │
│              CLI args + Transport (stdio / HTTP)          │
│                                                           │
│   ┌─────────────────────┐    ┌──────────────────────┐    │
│   │   MCP Server        │    │  Hook HTTP Server    │    │
│   │   (stdio / HTTP)    │    │  (--hook-port 19533) │    │
│   │                     │    │                      │    │
│   │  store_qa           │    │  GET /api/recall     │    │
│   │  query_qa           │    │  ?q=...&context=...  │    │
│   │  merge_knowledge    │    │                      │    │
│   │  knowledge://       │    └──────────┬───────────┘    │
│   └────────┬────────────┘               │                │
│            │                            │                │
│   ┌────────▼────────────────────────────▼───────────┐    │
│   │          embedding.rs + storage.rs              │    │
│   │    ONNX Runtime (384-dim) + LanceDB (3 tables)  │    │
│   └─────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────┘
         ▲                              ▲
         │ MCP (stdio/HTTP)             │ HTTP GET
         │                              │
┌────────┴────────┐          ┌──────────┴──────────┐
│  AI Client      │          │  Hook Script        │
│  (Claude Code,  │          │  memorize-hook.mjs  │
│   Gemini CLI,   │          │  (UserPromptSubmit  │
│   OpenCode)     │          │   / BeforeAgent)    │
└─────────────────┘          └─────────────────────┘
```

The system consists of two parts:

- **MCP Server** — Standard MCP protocol, providing store_qa / query_qa / merge_knowledge tools for the AI agent to store and retrieve memories
- **Hook HTTP Server** — Lightweight HTTP endpoint (`/api/recall`), called by hook scripts on each user question to inject relevant memories into the system prompt

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust (edition 2024) |
| MCP SDK | rmcp v0.15 (stdio + Streamable HTTP) |
| Hook Server | axum (HTTP) |
| Vector Store | LanceDB v0.26 (local embedded) |
| Embedding Inference | ONNX Runtime v1.23+ (via ort crate, dynamically loaded) |
| Tokenizer | tokenizers v0.21 |
| Vector Dimensions | 384 |
| npm Distribution | Platform packages (esbuild-style optional dependencies) |

## Highlights

- **Fully Local & Free** — All inference and storage on your machine, no cloud services, no API keys, no fees
- **Auto Memory Recall** — Hook scripts integrate into client workflows, automatically injecting relevant memories each conversation
- **Shareable Knowledge** — All memories export to human-readable `memorize_data.json` for team sharing, cross-device sync, or version control
- **Non-invasive Integration** — Plugs in via native Plugin/Hook/Extension mechanisms without modifying the client itself
- **Semantic Merging** — Similar QA pairs automatically merge into refined knowledge entries

## Project Structure

```
memorize_mcp/
├── src/
│   ├── main.rs             # CLI entry + transport layer
│   ├── server.rs           # MCP server (3 tools + 1 resource)
│   ├── hook.rs             # Hook HTTP Server (axum, /api/recall)
│   ├── embedding.rs        # ONNX inference engine
│   ├── storage.rs          # LanceDB storage layer (3 tables)
│   ├── persistence.rs      # JSON snapshot export + bidirectional sync on startup
│   ├── transport.rs        # Resilient stdio transport
│   ├── models.rs           # Data models + constants
│   └── lib.rs              # Module exports
├── hooks/
│   ├── memorize-hook.mjs       # Hook script (Claude Code / Gemini CLI)
│   ├── opencode-plugin.mjs     # OpenCode plugin
│   ├── hooks.json              # Claude Code Plugin hooks definition
│   ├── claude-code-settings.json
│   ├── gemini-cli-settings.json
│   └── opencode-config.json
├── npm/
│   ├── qa-memorize-mcp/            # Main package (bin/run.js entry)
│   ├── qa-memorize-mcp-win-x64/    # Windows platform package
│   ├── qa-memorize-mcp-linux-x64/  # Linux platform package
│   ├── qa-memorize-mcp-darwin-x64/ # macOS Intel platform package
│   └── qa-memorize-mcp-darwin-arm64/ # macOS ARM platform package
├── scripts/
│   ├── package.py          # Cross-platform packaging
│   ├── bump-version.js     # Unified version management
│   ├── compress-model.mjs  # Model gzip compression
│   ├── pack-npm.py         # npm platform package assembly
│   └── publish.py          # Manual npm publish
├── .github/workflows/
│   └── npm-publish.yml     # CI: 4-platform build + npm publish
├── .claude-plugin/         # Claude Code Plugin manifest
├── commands/recall.md      # /recall slash command
├── gemini-extension/       # Gemini CLI Extension config
├── .claude-plugin/         # Claude Code Plugin (marketplace.json + plugin.json)
└── tests/
    ├── integration.rs      # End-to-end integration tests
    ├── test_npm_package.py # npm package e2e tests
    └── test_hooks.py       # Hook system tests
```

## Data Storage & Sharing

Default data directory is `~/.memorize-mcp/` (override with `--db-path`).

```
~/.memorize-mcp/
├── *.lance, ...           # LanceDB persistence files
└── memorize_data.json     # Human-readable JSON snapshot
```

`memorize_data.json` is a complete snapshot of all memories. You can:
- Share between team members (just copy the file)
- Sync across devices (put in Dropbox / iCloud / Git repo)
- Version control to track knowledge evolution

Bidirectional sync on startup: records in JSON but not in LanceDB are written to LanceDB, and vice versa. Even if LanceDB files are corrupted, recovery from JSON is possible.

## Build from Source

```bash
# Prerequisites: Rust toolchain, ONNX Runtime >= 1.23, embedding_model/
cargo build --release
./target/release/memorize_mcp --hook-port 19533
```

## Tests

```bash
cargo test -- --test-threads=1          # All tests
cargo test --lib -- --test-threads=1    # Unit tests only
cargo test --test integration -- --test-threads=1  # Integration tests only
```

## License

LGPL-3.0-or-later
