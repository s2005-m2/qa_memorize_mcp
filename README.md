# Memorize MCP

基于 RAG 的记忆 MCP 服务器。通过本地 ONNX 模型进行文本向量化，使用 LanceDB 存储 QA 对和知识条目，支持语义检索与 LLM 驱动的知识融合。

## 架构

```
┌─────────────────────────────────────────────────┐
│                   main.rs                        │
│         CLI args + Transport (stdio / HTTP)      │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────┐
│                  server.rs                       │
│   MemorizeServer: 3 Tools + 1 Resource Template  │
│   ┌─────────┐ ┌──────────┐ ┌────────────────┐   │
│   │store_qa │ │ query_qa │ │merge_knowledge │   │
│   └────┬────┘ └────┬─────┘ └───────┬────────┘   │
│        │           │               │ (sampling)  │
│   ┌────▼───────────▼───────────────▼────────┐    │
│   │  knowledge://{topic}/{query} resource   │    │
│   └─────────────────────────────────────────┘    │
└──────────┬──────────────────────┬────────────────┘
           │                      │
┌──────────▼──────────┐ ┌────────▼─────────────┐
│    embedding.rs     │ │     storage.rs       │
│  ONNX Runtime + tok │ │  LanceDB 3 tables    │
│  text → Vec<f32>    │ │  topics/qa/knowledge │
└─────────────────────┘ └──────────────────────┘
```

## 技术栈

| 组件 | 技术 |
|------|------|
| 语言 | Rust (edition 2024) |
| MCP SDK | rmcp v0.15 (stdio + Streamable HTTP) |
| 向量存储 | LanceDB v0.26 (本地嵌入式) |
| Embedding 推理 | ONNX Runtime v1.23+ (通过 ort crate, 动态加载) |
| Tokenizer | tokenizers v0.21 |
| 向量维度 | 384 维 |

## MCP 工具

### store_qa

存储一条 QA 对。主题会自动按语义去重——如果已有相似主题则复用。

```json
{
  "question": "Rust 的所有权机制是什么？",
  "answer": "Rust 通过所有权系统在编译期管理内存，无需垃圾回收。",
  "topic": "Rust 编程"
}
```

### query_qa

根据问题和上下文语义检索相关 QA 对。先用 context 匹配主题，再在主题内搜索。

```json
{
  "question": "Rust 如何管理内存？",
  "context": "编程语言"
}
```

### merge_knowledge

扫描指定主题下的相似 QA 对，通过 MCP Sampling 请求 LLM 将它们融合为精炼的知识条目。融合后的 QA 标记为 merged，不再出现在搜索结果中。

```json
{
  "topic": "Rust 编程",
  "threshold": 0.85
}
```

## MCP 资源模板

### knowledge://{topic}/{query}

按主题和查询语义检索已融合的知识条目，可用于自动注入上下文。

## 快速开始

### 前置条件

- Rust toolchain (edition 2024)
- ONNX Runtime >= 1.23.0（放在可执行文件旁边，或通过 `pip install onnxruntime` 安装）
- Embedding 模型文件：`embedding_model/model_ort.onnx` + `embedding_model/tokenizer.json`

### 构建

```bash
cargo build --release
```

### 运行

stdio 模式（用于 Claude Desktop 等 MCP 客户端）：

```bash
./target/release/memorize_mcp
```

HTTP 模式：

```bash
./target/release/memorize_mcp --transport http --port 19532
```


### 命令行参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--transport` | `stdio` | 传输模式：`stdio` 或 `http` |
| `--port` | `19532` | HTTP 模式监听端口 |
| `--db-path` | `~/.memorize-mcp` | LanceDB 数据库路径及 JSON 快照目录 |
| `--model-dir` | `./embedding_model` | Embedding 模型目录 |

### Claude Desktop 配置

```json
{
  "mcpServers": {
    "memorize": {
      "command": "/path/to/memorize_mcp"
    }
  }
}
```

### OpenCode 配置

在项目根目录的 `opencode.json` 或全局 `~/.config/opencode/opencode.json` 中添加：

stdio 模式（本地运行）：

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["/path/to/memorize_mcp"],
      "environment": {
        "ORT_DYLIB_PATH": "/path/to/libonnxruntime.so"
      },
      "enabled": true
    }
  }
}
```

HTTP 模式（远程连接）：

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "remote",
      "url": "http://localhost:19532/sse",
      "enabled": true
    }
  }
}
```

`environment` 可选，仅在需要手动指定 ONNX Runtime 路径时使用。

不指定 `--db-path` 时，数据自动存储在 `~/.memorize-mcp/`。

## npm 安装

```bash
# 直接运行（自动下载）
npx qa-memorize-mcp

# 全局安装
npm install -g qa-memorize-mcp
qa-memorize-mcp
```

### 客户端配置

#### Claude Code

Plugin 一键安装（推荐）：
```
/plugin marketplace add s2005-m2/memorize_mcp
/plugin install qa-memorize-mcp@qa-memorize-mcp
```

或手动添加 MCP：
```bash
claude mcp add memorize -- npx -y qa-memorize-mcp --hook-port 19533
```

#### Gemini CLI

在 `.gemini/settings.json` 中添加：
```json
{
  "mcpServers": {
    "qa-memorize-mcp": {
      "command": "npx",
      "args": ["-y", "qa-memorize-mcp", "--hook-port", "19533"]
    }
  }
}
```

#### OpenCode

在 `opencode.json` 中添加：
```json
{
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["npx", "-y", "qa-memorize-mcp", "--hook-port", "19533"],
      "enabled": true
    }
  }
}
```

## 打包分发

使用 `scripts/package.py` 将可执行文件、ONNX Runtime 动态库和模型文件打包到一个目录：

```bash
# 自动检测当前平台，构建并打包
python scripts/package.py --build

# 指定目标平台
python scripts/package.py --platform osx-arm64 --output dist/macos

# 仅打包（已构建好的情况下）
python scripts/package.py --output dist
```

支持的平台：

| 平台 | 标识 |
|------|------|
| Windows x64 | `win-x64` |
| Linux x64 | `linux-x64` |
| macOS x86_64 | `osx-x86_64` |
| macOS ARM64 | `osx-arm64` |

打包输出结构：

```
dist/
├── memorize_mcp(.exe)
├── onnxruntime.dll / libonnxruntime.dylib / libonnxruntime.so
├── embedding_model/
│   ├── model_ort.onnx
│   └── tokenizer.json
└── hooks/
    ├── memorize-hook.mjs       # Claude Code / Gemini CLI hook
    ├── opencode-plugin.mjs     # OpenCode 插件
    ├── claude-code-settings.json
    ├── gemini-cli-settings.json
    └── opencode-config.json
```

## ONNX Runtime 查找顺序

程序启动时按以下顺序查找 ONNX Runtime 动态库：

1. `ORT_DYLIB_PATH` 环境变量（开发/CI 用）
2. 可执行文件同目录（生产部署）
3. Python `onnxruntime` 包（开发回退）

## 项目结构

```
memorize_mcp/
├── Cargo.toml
├── src/
│   ├── lib.rs              # 模块导出
│   ├── main.rs             # CLI 入口 + 传输层
│   ├── server.rs           # MCP 服务器 (3 tools + 1 resource)
│   ├── embedding.rs        # ONNX 推理引擎
│   ├── storage.rs          # LanceDB 存储层 (3 tables)
│   ├── persistence.rs      # JSON 快照导出 + 启动时双向同步
│   ├── transport.rs        # Resilient stdio transport
│   └── models.rs           # 数据模型 + 常量
├── tests/
│   └── integration.rs      # 端到端集成测试
├── scripts/
│   └── package.py          # 跨平台打包脚本
├── hooks/
│   ├── memorize-hook.mjs       # Claude Code / Gemini CLI hook (Node.js)
│   ├── opencode-plugin.mjs     # OpenCode 插件
│   ├── claude-code-settings.json
│   ├── gemini-cli-settings.json
│   └── opencode-config.json
└── embedding_model/
    ├── model_ort.onnx       # ONNX 模型
    └── tokenizer.json       # Tokenizer
```

## 测试

```bash
# 全部测试（单元 + 集成）
cargo test -- --test-threads=1

# 仅单元测试
cargo test --lib -- --test-threads=1

# 仅集成测试
cargo test --test integration -- --test-threads=1
```

需要 `--test-threads=1` 以确保 LanceDB 临时目录隔离。

## 数据存储

默认数据目录为 `~/.memorize-mcp/`（可通过 `--db-path` 覆盖）。

### LanceDB 表

| 表名 | 字段 | 说明 |
|------|------|------|
| `topics` | topic_name, vector | 主题及其向量表示 |
| `qa_records` | question, answer, topic, merged, vector | QA 对 |
| `knowledge` | knowledge_text, topic, source_questions, vector | 融合后的知识条目 |

### 数据持久化与同步

数据目录下同时维护两份数据：

```
~/.memorize-mcp/
├── *.lance, ...           # LanceDB 持久化文件（程序直接读写）
└── memorize_data.json     # 人类可读的 JSON 快照（可分享/版本控制）
```

**关闭时**：MCP 服务退出前自动将 3 张表全量导出为 `memorize_data.json`，包含所有记录及其向量。

**启动时**：如果存在 `memorize_data.json`，执行双向同步：
- JSON 中有而 LanceDB 中没有的记录 → 使用已有向量（或重新 embed）写入 LanceDB
- LanceDB 中有而 JSON 中没有的记录 → 重新导出更新 JSON

这确保了即使 LanceDB 文件损坏或丢失，也可以从 JSON 快照恢复；反之亦然。

## License

MIT
