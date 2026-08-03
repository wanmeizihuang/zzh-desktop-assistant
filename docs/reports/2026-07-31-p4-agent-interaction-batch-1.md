# P4 智能体交互第一批报告

日期：2026-07-31
范围：冻结流式事件顺序与运行状态语义。

## 实现

- 新增 `AgentEventKind`，为五类传输无关事件提供稳定分类。
- `AgentEvent` 提供请求 ID 与事件类型读取，不暴露传输细节。
- 新增 `RunPhase`：等待开始、流式响应、完成、取消和失败。
- 新增 `RunTranscript`，累积同一请求的响应文本并保留失败详情。
- 新增 `EventSequenceError`，拒绝跨请求事件和当前阶段不允许的事件。
- 非法事件在修改状态前返回，避免旧连接器事件污染新会话。
- 失败状态根据机器可读错误码提供重试资格；取消和成功终态不可重试。

## 自动化验证

新增 6 项测试覆盖：

- `Started → TextDelta* → Completed` 合法流式序列。
- 跨请求 ID 事件拒绝且状态不变。
- 开始前文本拒绝。
- 重复开始拒绝。
- 取消与失败终态详情。
- 终态后事件拒绝。

质量门禁：

- `cargo test -p agent-core --offline`：13 项通过。
- `cargo test --workspace --offline`：99 项通过，1 项隔离 HKCU 测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。

## 界面验证

本批不修改桌面界面，无人工界面步骤。后续对话 UI 接入时只提供人工验证清单，不自动操作用户桌面。

## 下一步

进入 Task 2：实现连接器目录和不包含秘密的 HTTP/CLI 配置模型，为智能体切换、OpenAI 兼容 API 与 Codex CLI 接入建立稳定配置边界。
