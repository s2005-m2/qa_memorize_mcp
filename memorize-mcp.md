# Draft: Memorize MCP - RAG-based Memory MCP Server

## Requirements (confirmed)

### 技术栈
- **语言**: Rust (edition 2024)
- **MCP SDK**: rmcp v0.15.0 (stdio + SSE 双模式)
- **向量存储**: LanceDB v0.26.2 (本地嵌入式)
- **Embedding推理**: candle-core + candle-nn + candle-onnx (纯Rust, 无外部DLL) + tokenizers v0.21.0
- **模型**: GTE-multilingual-base (Gemma2架构), FP16 ONNX格式, 768维输出, 262K词表

### 核心数据模型
- 两种记录类型:
  1. **QA记录**: question + answer + topic + question_vector (768维)
  2. **知识记录**: knowledge_text + topic + knowledge_vector (768维)
- 主题(topic)作为逻辑分区字段

### 主题分区机制
- **动态生成**: AI(LLM)给出新主题名
- **去重**: 新主题与已有主题做向量相似度比较，过于相似则复用已有主题
- **存储**: 主题本身也需要有向量表示用于匹配

### 查询流程
- **流程A (问题+背景)**: 背景embedding → 向量匹配主题 → 问题embedding在该主题分区内搜索 → 返回答案
- **流程B (LLM直接输入)**: 输入embedding → 匹配主题 → 输入在主题内搜索知识 → 返回

### 存储流程
- 输入: (question, answer) 或 知识文本
- question embedding → 存入LanceDB，带topic标签
- 可触发问题融合

### 问题融合
- **触发**: store_qa 存储时，对同topic下的QA做相似度检测，超过阈值则通知客户端可融合
- **执行**: 由LLM通过 MCP sampling 进行融合（不是程序自动合并）
- **结果**: 相似问题合并为知识记录，原QA标记merged

### Transport
- stdio + SSE 双模式支持

## Technical Decisions
- LanceDB `.only_if("topic = 'xxx'")` 用于主题分区过滤
- 本地ONNX推理(candle-onnx)，无需外部embedding API，无需外部DLL
- 主题匹配用向量相似度（背景text → embedding → 与主题向量比较）

## Research Findings
- **Embedding模型**: GTE-multilingual-base, 768维, 25层Transformer, FP16 ONNX格式(标准算子)
- **rmcp**: v0.15.0, #[tool_router] + #[tool] 宏模式, ServerHandler trait
- **LanceDB**: v0.26.2, Arrow schema, FixedSizeList向量列, .only_if()过滤
- **candle-onnx**: v0.9.1, `candle_onnx::read_file()` 加载模型, `candle_onnx::simple_eval()` 推理
- **tokenizers**: v0.21.0, Tokenizer::from_file() 加载tokenizer.json

## Decisions (Round 2)

### LLM融合执行方
- **通过 MCP sampling 请求**: server 检测到可融合问题后，通过 MCP 协议的 createMessage/sampling 能力请求客户端 LLM 执行融合
- server 本身不持有 LLM client，不直接调用外部 LLM API

### MCP Tool 划分
- **store_qa**: 主动触发，存储 (question, answer) 对
- **query_qa**: 主动触发，问题+背景查询QA
- **query_knowledge**: 被动/自动触发，所有给大模型的输出都过一遍（类似传统RAG的自动检索）

### 数据表结构
- **topics 表**: 独立存储，topic_name + topic_vector (768维)，用于主题匹配
- **QA 表 vs 知识表**: 分两张表，查询模式不同

### query_knowledge 的特殊性
- 这不是用户主动调用的 tool，而是类似 RAG 的自动检索
- 所有 LLM 输出/输入都应该经过这个检索
- 实现为 MCP resource，客户端订阅后自动获取

## Decisions (Round 3)

### query_knowledge 触发方式
- **MCP Resource 自动注入**: 不是 tool，而是 MCP resource
- 客户端自动获取相关知识上下文，注入到 LLM 对话中
- 实现方式: MCP resource template，客户端订阅后自动获取

### topic 来源
- **调用方显式传入**: store_qa 的参数中必须包含 topic
- server 负责: 将传入的 topic 与已有 topics 做向量相似度比较
  - 相似度高 → 复用已有 topic
  - 相似度低 → 创建新 topic 记录

### 融合后数据形态
- **QA 保留(标记merged) + 生成 knowledge 记录**
- 原始 QA 记录标记 merged=true，不再参与搜索
- 生成新的 knowledge 记录存入 knowledge 表
- knowledge 记录: knowledge_text(LLM融合结果) + topic + vector

### 主题匹配失败处理
- **不做 fallback**: 如果背景无法匹配主题，返回空
- 由 LLM 自行解决问题，然后通过 store_qa 存储新知识
- 这意味着系统是"冷启动"的，需要先积累数据

## Final Architecture Summary

### Embedding推理方案
- **引擎**: candle-onnx (纯Rust, 无外部DLL依赖)
- **模型格式**: FP16 ONNX (标准ONNX算子, candle-onnx完全支持)
- **加载**: `candle_onnx::read_file("model.onnx")` → ModelProto
- **推理**: `candle_onnx::simple_eval(&model, inputs)` → HashMap<String, Tensor>
- **输入**: input_ids(i64) + attention_mask(i64), shape [1, seq_len]
- **输出**: last_hidden_state [1, seq_len, 768] → mean pooling → L2 normalize → 768维向量
- **tokenizer**: tokenizers v0.21.0, Tokenizer::from_file() 加载tokenizer.json

### Cargo依赖
```toml
candle-core = "0.9.2"
candle-nn = "0.9.2"
candle-onnx = "0.9.1"
tokenizers = "0.21.0"
```

### LanceDB 表结构 (3张表)
1. **topics**: topic_name(Utf8) + topic_description(Utf8) + vector(768维) 
2. **qa_records**: question(Utf8) + answer(Utf8) + topic(Utf8) + merged(Bool) + vector(768维)
3. **knowledge**: knowledge_text(Utf8) + topic(Utf8) + source_questions(List<Utf8>) + vector(768维)

### MCP Tools (3个)
1. **store_qa(question, answer, topic)**: 存储QA → embedding → 主题去重 → 存入qa_records → 同topic下相似度检测，超阈值通知可融合
2. **query_qa(question, context)**: context embedding → 匹配topic → question embedding在topic内搜索qa_records → 返回answers
3. **merge_knowledge(topic?, threshold?)**: 触发融合 → 找相似QA → MCP sampling请求LLM融合 → 生成knowledge记录 → 标记原QA

### MCP Resource (1个)
4. **knowledge://{topic}/{query}**: 自动注入知识上下文，客户端订阅后自动获取相关knowledge

### MCP Sampling
- merge_knowledge 执行时，通过 MCP sampling/createMessage 请求客户端 LLM 执行融合

### QA融合触发机制
- **store_qa 时自动检测**: 存储新QA后，在同topic下搜索相似QA（向量相似度 > 阈值）
- **检测到可融合项**: store_qa 返回结果中附带融合建议（相似QA列表 + 相似度分数）
- **客户端决策**: 客户端收到建议后决定是否调用 merge_knowledge 执行融合
- **手动触发**: 也可直接调用 merge_knowledge(topic, threshold) 扫描指定topic下的可融合QA

## Scope Boundaries
- INCLUDE: MCP server, embedding推理(candle-onnx+tokenizers), LanceDB存储/查询, 主题管理, 问题融合触发, MCP sampling请求, stdio+Streamable HTTP transport
- EXCLUDE: LLM推理引擎本身, 前端UI, 模型训练, 外部LLM API调用, 外部DLL依赖
