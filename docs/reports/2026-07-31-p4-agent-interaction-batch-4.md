# P4 智能体交互第四批报告

日期：2026-07-31
范围：Codex CLI 可执行文件探测、固定参数、JSONL 流式输出与终止语义。

## 实现

- 新增 `CodexCliConfig`，支持显式可执行文件路径和从 `PATH` 顺序探测 `codex.exe`/`codex`；缺失程序在创建连接器前返回配置错误。
- 新增稳定 ID 为 `codex-cli` 的 `CodexCliConnector`，复用现有按需工作线程和安全子进程执行器。
- 固定以参数数组构造 `codex exec --json --skip-git-repo-check --sandbox read-only -`，附加参数保持独立字面参数，提示词只经 stdin 传入，不进行 shell 拼接。
- Windows 子进程继续使用 `CREATE_NO_WINDOW`，不会弹出控制台窗口。
- JSONL 适配器识别 `item.updated`、`item.completed`、`turn.completed`、`turn.failed` 和顶层错误事件。
- 同一智能体消息的完整快照会按已输出前缀转换为增量，避免 `item.updated` 与 `item.completed` 重复显示。
- 非 JSON、缺少必要字段、消息回退覆盖、成功退出但缺少 `turn.completed` 均映射为协议错误。
- 取消继续杀死并等待子进程；stderr 最多读取 8 KiB，非零退出保持进程错误及退出码。

## 协议核对

- 当前 Windows 商店版 Codex 路径可由系统命令发现。
- 受当前执行权限限制，无法直接运行该商店路径下的 `codex.exe --help`。
- OpenAI 在线 Codex 手册获取因官方端返回 HTTP 403 未成功。
- 对本机 Codex 26.727 二进制做只读字符串核对，确认包含 `skip-git-repo-check`、`thread.started`、`turn.started`、`item.started`、`item.updated`、`item.completed`、`turn.completed`、`turn.failed` 和 `agent_message` 标识。
- 自动化测试只运行本地 fixture，不发起真实 Codex 对话，不读取登录状态或凭据。

## 自动化验证

`agent-cli` 本地真实子进程测试覆盖：

- 搜索目录顺序和可执行文件发现。
- 固定 Codex 参数及 shell 元字符的字面传递。
- JSONL 消息更新的连续增量和去重。
- Codex 失败事件到协议错误的映射。
- 取消后终止子进程。
- 缺失 Codex 可执行文件的同步配置错误。
- 原有通用 CLI 流式输出、stdin 元字符、非零退出、有限 stderr 和取消回归。

质量门禁：

- `cargo test -p agent-cli --offline`：11 项通过。
- `cargo test --workspace --offline`：124 项通过，1 项隔离 HKCU 测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
- `cargo build --release -p desktop-assistant --offline`：通过。
- `cargo fmt --all -- --check`：通过。

## 界面验证

本批未接入桌面对话界面，不需要人工界面验证，也没有自动启动或控制用户桌面。

## 下一步

进入 P4 Task 5：实现 Windows Credential Manager 的当前用户 Generic Credential 读写、删除、命名验证和秘密脱敏测试。
