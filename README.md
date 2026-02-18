# Memorize MCP

AI 编程助手的跨会话记忆系统。通过 MCP 协议存储和检索 QA 对与知识条目，配合 Hook 机制在每次对话开始时自动召回相关记忆。

支持 Claude Code、Gemini CLI、OpenCode 三个客户端，通过 npm 一键安装。

## 架构

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

系统由两部分组成：

- **MCP Server** — 标准 MCP 协议，提供 store_qa / query_qa / merge_knowledge 三个工具，AI 助手通过它存储和检索记忆
- **Hook HTTP Server** — 轻量 HTTP 端点 (`/api/recall`)，Hook 脚本在每次用户提问时调用它，将相关记忆注入到 system prompt 中

## 技术栈

| 组件 | 技术 |
|------|------|
| 语言 | Rust (edition 2024) |
| MCP SDK | rmcp v0.15 (stdio + Streamable HTTP) |
| Hook Server | axum (HTTP) |
| 向量存储 | LanceDB v0.26 (本地嵌入式) |
| Embedding 推理 | ONNX Runtime v1.23+ (通过 ort crate, 动态加载) |
| Tokenizer | tokenizers v0.21 |
| 向量维度 | 384 维 |
| npm 分发 | 平台包模式 (esbuild-style optional dependencies) |

## 特性

- **完全本地、完全免费** — 所有推理和存储都在本机完成，无需云服务、无需 API key、无需付费
- **自动记忆召回** — Hook 脚本嵌入客户端工作流，每次对话自动注入相关记忆，无需手动操作
- **知识可分享** — 所有记忆导出为人类可读的 `memorize_data.json`，可在团队成员间分享、跨设备同步、纳入版本控制
- **无侵入嵌入** — 通过客户端原生的 Plugin/Hook/Extension 机制接入，不修改客户端本身
- **语义融合** — 相似 QA 自动融合为精炼知识条目，记忆越用越精准

## 快速开始

```bash
npx qa-memorize-mcp
```

一行命令启动。自动下载平台原生二进制 + ONNX Runtime + Embedding 模型，无需 Rust 工具链。

### 客户端配置

#### Claude Code

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

### Hook 配置（自动召回）

MCP 服务启动后，配置客户端 Hook 即可实现每次对话自动召回相关记忆：

- **Claude Code** — 将 `hooks/claude-code-settings.json` 合并到 `.claude/settings.json`，或使用 Plugin 一键安装
- **Gemini CLI** — 将 `hooks/gemini-cli-settings.json` 合并到 `.gemini/settings.json`
- **OpenCode** — 将 `hooks/opencode-config.json` 合并到 `opencode.json`

Hook 脚本 (`memorize-hook.mjs`) 在用户每次提问时调用 `/api/recall` 端点，将语义匹配的 QA 和知识条目注入到 system prompt。

## MCP 工具

### store_qa

存储一条 QA 对。主题自动按语义去重。

```json
{ "question": "Rust 的所有权机制是什么？", "answer": "Rust 通过所有权系统在编译期管理内存。", "topic": "Rust 编程" }
```

### query_qa

语义检索相关 QA 对。先用 context 匹配主题，再在主题内搜索。

```json
{ "question": "Rust 如何管理内存？", "context": "编程语言" }
```

### merge_knowledge

扫描相似 QA 对，通过 MCP Sampling 请求 LLM 融合为精炼知识条目。

```json
{ "topic": "Rust 编程", "threshold": 0.85 }
```

### knowledge://{topic}/{query}

资源模板。按主题和查询语义检索已融合的知识条目。

## 命令行参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--transport` | `stdio` | 传输模式：`stdio` 或 `http` |
| `--port` | `19532` | HTTP 模式监听端口 |
| `--hook-port` | _(关闭)_ | Hook HTTP Server 端口（启用后提供 `/api/recall`） |
| `--db-path` | `~/.memorize-mcp` | LanceDB 数据库路径及 JSON 快照目录 |
| `--model-dir` | `./embedding_model` | Embedding 模型目录 |
| `--debug` | _(关闭)_ | 输出调试日志到 stderr |

## 项目结构

```
memorize_mcp/
├── src/
│   ├── main.rs             # CLI 入口 + 传输层
│   ├── server.rs           # MCP 服务器 (3 tools + 1 resource)
│   ├── hook.rs             # Hook HTTP Server (axum, /api/recall)
│   ├── embedding.rs        # ONNX 推理引擎
│   ├── storage.rs          # LanceDB 存储层 (3 tables)
│   ├── persistence.rs      # JSON 快照导出 + 启动时双向同步
│   ├── transport.rs        # Resilient stdio transport
│   ├── models.rs           # 数据模型 + 常量
│   └── lib.rs              # 模块导出
├── hooks/
│   ├── memorize-hook.mjs       # Hook 脚本 (Claude Code / Gemini CLI)
│   ├── opencode-plugin.mjs     # OpenCode 插件
│   ├── hooks.json              # Claude Code Plugin hooks 定义
│   ├── claude-code-settings.json
│   ├── gemini-cli-settings.json
│   └── opencode-config.json
├── npm/
│   ├── qa-memorize-mcp/            # 主包 (bin/run.js 入口)
│   ├── qa-memorize-mcp-win-x64/    # Windows 平台包
│   ├── qa-memorize-mcp-linux-x64/  # Linux 平台包
│   ├── qa-memorize-mcp-darwin-x64/ # macOS Intel 平台包
│   └── qa-memorize-mcp-darwin-arm64/ # macOS ARM 平台包
├── scripts/
│   ├── package.py          # 跨平台打包
│   ├── bump-version.js     # 版本号统一管理
│   ├── compress-model.mjs  # 模型 gzip 压缩
│   ├── pack-npm.py         # npm 平台包组装
│   └── publish.py          # 手动 npm 发布
├── .github/workflows/
│   └── npm-publish.yml     # CI: 4 平台构建 + npm 发布
├── .claude-plugin/         # Claude Code Plugin manifest
├── commands/recall.md      # /recall slash command
├── gemini-extension/       # Gemini CLI Extension 配置
├── marketplace.json        # Claude Code Plugin Marketplace
└── tests/
    ├── integration.rs      # Rust 端到端集成测试
    ├── test_npm_package.py # npm 包端到端测试
    └── test_hooks.py       # Hook 系统功能测试
```

## 数据存储与分享

默认数据目录为 `~/.memorize-mcp/`（可通过 `--db-path` 覆盖）。

```
~/.memorize-mcp/
├── *.lance, ...           # LanceDB 持久化文件
└── memorize_data.json     # 人类可读的 JSON 快照
```

`memorize_data.json` 是所有记忆的完整快照，可以：
- 在团队成员间分享（复制文件即可）
- 跨设备同步（放入 Dropbox / iCloud / Git 仓库）
- 纳入版本控制，追踪知识演变

启动时自动双向同步：JSON 中有而 LanceDB 中没有的记录写入 LanceDB，反之亦然。即使 LanceDB 文件损坏，也可从 JSON 恢复。

## 从源码构建

```bash
# 前置条件：Rust toolchain, ONNX Runtime >= 1.23, embedding_model/
cargo build --release
./target/release/memorize_mcp --hook-port 19533
```

## 测试

```bash
cargo test -- --test-threads=1          # 全部测试
cargo test --lib -- --test-threads=1    # 仅单元测试
cargo test --test integration -- --test-threads=1  # 仅集成测试
```

## License

MIT
