# QA Memorize MCP

为什么叫 QA Memorize？因为它做的事情就是：**记住 Agent 解决问题的经验**(Question & Answer)。

每次你和 Agent 编程助手对话，解决了一个问题、踩了一个坑、搞清了一个概念——这些经验默认会随着会话关闭而消失。QA Memorize MCP 把这些 QA 对存下来，积累的QA会转换为知识条目，下次遇到相似问题时自动召回，并支持直接查询QA，让 Agent 越用越聪明。

## 功能

- **存储 QA 对** — Agent 助手在对话中将有价值的问答存入本地向量数据库，按主题自动归类
- **语义检索** — 根据语义相似度（而非关键词）检索相关记忆
- **知识融合** — 相似 QA 自动融合为精炼知识条目，记忆越用越精准
- **自动召回** — 通过 Hook 机制，每次新对话自动注入相关记忆到 system prompt，无需手动操作
- **知识可分享** — 所有记忆导出为 `memorize_data.json`，可团队分享、跨设备同步、纳入版本控制
- **完全本地、完全免费** — 内置轻量级 Embedding 模型（384 维）和嵌入式向量数据库 LanceDB，所有推理和存储都在本机完成，无需云服务、无需 API key，数据永远不离开你的电脑

## 客户端集成

#### Claude Code

**Plugin 一键安装（推荐）：**

```bash
/plugin marketplace add s2005-m2/qa_memorize_mcp
/plugin install qa-memorize-mcp@s2005-m2
```

#### Gemini CLI

**Extension 一键安装（推荐）：**

```bash
gemini extensions install https://github.com/s2005-m2/qa_memorize_mcp
```


#### OpenCode

**面向 Agent 配置（推荐）：**
```agent
使用 `fetch` 或 `curl` 获取 https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/docs/opencode-agent-setup.md 的内容，阅读后按其中的步骤为用户完成配置。
```

## 下载 MCP

```bash
npx qa-memorize-mcp
```

一行命令启动。自动下载平台原生二进制 + ONNX Runtime + Embedding 模型，无需 Rust 工具链。

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

## 自动召回 API

### GET /api/recall

Hook 脚本调用的 HTTP 端点，根据用户提问语义匹配主题并检索已融合的知识条目，供注入 system prompt。需通过 `--hook-port` 启用。

**请求参数（查询字符串）：**

| 参数 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `context` | 是 | — | 用户的原始提问文本，用于匹配主题并在主题内检索知识 |
| `limit` | 否 | `5` | 返回结果数量上限 |

**返回值：** JSON 数组，每个元素为知识条目：

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

`score` 为 L2 距离（越小越相似）。无匹配时返回空数组 `[]`。

**召回流程：**

```
用户输入提问
    │
    ▼
客户端 Hook 触发
    ├─ Claude Code: UserPromptSubmit 事件 → memorize-hook.mjs
    ├─ Gemini CLI:  BeforeAgent 事件    → memorize-hook.mjs
    └─ OpenCode:    system.transform hook → opencode-plugin.mjs
    │
    │  GET http://localhost:19533/api/recall?context=<用户提问>&limit=5
    │  超时 2 秒，失败静默（不阻塞用户交互）
    │
    ▼
Hook 服务器处理 (hook.rs)
    context 向量化 → 匹配最相似主题 → 在该主题内检索 Knowledge
    │
    ▼
Hook 脚本格式化结果 → 注入 system prompt
    "[Memory Recall]\nKnowledge: ..."
```

## 命令行参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--transport` | `stdio` | 传输模式：`stdio` 或 `http` |
| `--port` | `19532` | HTTP 模式监听端口 |
| `--hook-port` | _(关闭)_ | Hook HTTP Server 端口（启用后提供 `/api/recall`） |
| `--db-path` | `~/.memorize-mcp` | LanceDB 数据库路径及 JSON 快照目录 |
| `--model-dir` | `./embedding_model` | Embedding 模型目录 |
| `--debug` | _(关闭)_ | 输出调试日志到 stderr |

## 架构

```
┌─────────────────────────────────────────────────────────┐
│                        main.rs                           │
│            命令行参数 + 传输层 (stdio / HTTP)             │
│                                                           │
│   ┌─────────────────────┐    ┌──────────────────────┐    │
│   │   MCP 服务器        │    │  Hook HTTP 服务器    │    │
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
│   │    ONNX Runtime (384维) + LanceDB (3 张表)    │    │
│   └─────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────┘
         ▲                              ▲
         │ MCP (stdio/HTTP)             │ HTTP GET
         │                              │
┌────────┴────────┐          ┌──────────┴──────────┐
│  AI 客户端      │          │  Hook 脚本          │
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
| Hook 服务器 | axum (HTTP) |
| 向量存储 | LanceDB v0.26 (本地嵌入式) |
| Embedding 推理 | ONNX Runtime v1.23+ (通过 ort crate，动态加载) |
| 分词器 | tokenizers v0.21 |
| 向量维度 | 384 维 |
| npm 分发 | 平台包模式（esbuild 风格的可选依赖） |

## 特性

- **完全本地、完全免费** — 所有推理和存储都在本机完成，无需云服务、无需 API key、无需付费
- **自动记忆召回** — Hook 脚本嵌入客户端工作流，每次对话自动注入相关记忆，无需手动操作
- **知识可分享** — 所有记忆导出为人类可读的 `memorize_data.json`，可在团队成员间分享、跨设备同步、纳入版本控制
- **无侵入嵌入** — 通过客户端原生的 Plugin/Hook/Extension 机制接入，不修改客户端本身
- **语义融合** — 相似 QA 自动融合为精炼知识条目，记忆越用越精准

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
│   ├── transport.rs        # 健壮的 stdio 传输层
│   ├── models.rs           # 数据模型 + 常量
│   └── lib.rs              # 模块导出
├── hooks/
│   ├── memorize-hook.mjs       # Hook 脚本 (Claude Code / Gemini CLI)
│   ├── opencode-plugin.mjs     # OpenCode 插件
│   ├── hooks.json              # Claude Code Plugin 钩子定义
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
├── .claude-plugin/         # Claude Code Plugin 清单文件
├── commands/recall.md      # /recall 斜杠命令
├── gemini-extension/       # Gemini CLI 扩展配置
├── .claude-plugin/         # Claude Code Plugin (marketplace.json + plugin.json)
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
# 前置条件：Rust 工具链、ONNX Runtime >= 1.23、embedding_model/ 目录
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

LGPL-3.0-or-later
