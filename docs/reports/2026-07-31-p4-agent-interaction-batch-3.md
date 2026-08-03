# P4 智能体交互第三批报告

日期：2026-07-31
范围：OpenAI 兼容 HTTP SSE 流式连接器。

## 开始前备份

- 已创建 `backups/current-pre-p4-task3-20260731.zip`。
- 备份包含源码、文档、形象资源和根目录 EXE，排除 `.git`、`target` 与备份目录自身。
- 归档共 265 个条目，已逐项读取验证。
- 新备份确认后删除旧归档，当前只保留这一份备份。

## 实现

- 新增 `OpenAiCompatibleConfig`，显式配置 endpoint、model、连接/请求超时、SSE 字节上限和回答文本上限。
- 新增 `OpenAiCompatibleConnector`，复用一个 `reqwest` 客户端并使用 rustls，禁用默认原生 TLS 依赖。
- 每次请求按需创建工作线程和单线程 Tokio runtime，空闲时不增加常驻运行时。
- 请求使用 OpenAI Chat Completions 的 `model`、`stream: true` 与标准 role/content 消息结构。
- Bearer Token 保存在连接器私有字段，只写入 Authorization 请求头，不进入 `Debug`、错误信息或日志。
- 使用 `eventsource-stream` 解析标准 SSE，按顺序输出 `TextDelta`，仅在收到 `[DONE]` 后完成。
- 等待响应和每个 SSE 事件时同时检查取消信号；取消产生唯一 `Cancelled` 终态。
- SSE 总字节和累计回答文本分别受限，避免畸形或超长响应无界占用内存。
- 401/403、408/504、429、常见请求错误、传输错误和协议错误映射到稳定错误码。

## 自动化验证

本机脚本 HTTP 服务只监听 `127.0.0.1`，不访问外部网络，也不需要真实 API Key。测试覆盖：

- 有序 SSE 增量及 `[DONE]`。
- OpenAI 请求方法、路径、model、消息和 Bearer 请求头。
- 畸形 JSON 与敏感 Token 不泄漏。
- 收到部分文本后提前断流。
- 请求超时。
- 401 与 429 状态。
- 等待流数据时取消。
- SSE 总字节与累计回答文本超限。
- 无效 endpoint、空 model、零容量和空 Token 的构造拒绝。

质量门禁：

- `cargo test -p agent-http --offline`：12 项通过。
- `cargo test --workspace --offline`：117 项通过，1 项隔离 HKCU 测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
- `cargo build --release -p desktop-assistant --offline`：通过。

## 界面验证

本批未修改桌面对话界面，不需要人工界面验证，也没有自动启动或控制用户桌面。

## 下一步

进入 P4 Task 4：在现有安全子进程原型上完善 Codex CLI 可执行文件探测、参数构造、流式输出和终止语义。
