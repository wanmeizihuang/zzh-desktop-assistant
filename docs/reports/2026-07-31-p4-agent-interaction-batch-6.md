# P4 智能体交互第六批报告

日期：2026-07-31
范围：桌面对话 UI、连接器运行时桥接、停止与重试状态机。

## 实现

- 新增桌面端 `ConversationController`，只允许一个活动请求，并为发送和重试分配递增请求 ID。
- 对话历史仅驻留内存，限制为 24 条/256 KiB；提示限制 16 KiB，单次回答限制 64 KiB。
- 所有 `AgentEvent` 在进入 Slint 前先由 `RunTranscript` 校验请求 ID、事件顺序和终态。
- 对话页新增智能体滚动选择、虚拟消息列表、用户/智能体/错误气泡、状态提示、重试和发送/停止控件。
- 默认档案自动发现 `codex.exe`；配置中的 Codex 参数会去除重复的 `exec --json` 前缀。
- HTTP 档案支持无密钥本地端点，或通过凭据目标引用读取 Windows Credential Manager；缺失或无效档案仍显示在列表中并标记不可用。
- 活动请求期间禁止切换智能体；切换后持久化稳定档案 ID，并清除属于旧智能体的重试入口。
- 连接器工作完全位于 Slint 事件循环外；流式 UI 更新最高 20 FPS，终态立即刷新。
- 停止操作幂等；退出程序、回答超限、事件序列异常和运行句柄释放都会触发取消或受控关闭。
- 失败仅在错误码允许时显示重试；重试复用上一条用户消息但创建新请求 ID，旧事件无法污染新请求。

## 自动化验证

- `cargo test -p desktop-assistant --offline`：33 项通过，1 项隔离启动项测试按设计忽略。
- `cargo test --workspace --offline`：137 项通过，2 项系统状态测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过，零警告。
- `cargo build --release -p desktop-assistant --offline`：通过。
- `cargo fmt --all`：通过。
- 根目录 `desktop-assistant.exe` 已更新，大小 18,684,928 字节。

## 安全与资源边界

- API Key 不进入 Slint 模型、对话历史、JSON 配置或错误日志。
- HTTP/CLI 连接器只在用户发送请求时创建有限生命周期工作任务，空闲状态不增加常驻网络运行时。
- UI 只持有展示文本和档案状态；连接器与取消令牌由 Rust 运行时持有。
- 附件按钮保持禁用占位，P5 安全确认完成前不会读取或传递文件。

## 未自动执行

按用户确定的验证方式，本批没有启动、点击、截图或自动控制桌面程序。智能体菜单、中文输入、流式文本、停止、重试和长回答布局需要按 P4 关闭报告人工确认。

## 已知边界

- 当前设置页尚未提供档案/API Key 编辑器；HTTP 档案从版本 4 JSON 配置读取，凭据只接受 Windows Credential Manager 引用。
- 首期 CLI 档案仅支持稳定 ID `codex-cli` 的 Codex JSONL 协议；通用 CLI 和厂商预置属于后续能力。
- 文件拖放、附件确认、敏感文件阻止和厂商预置属于 P5。
