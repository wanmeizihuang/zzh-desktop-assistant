# P4 智能体交互第五批报告

日期：2026-07-31
范围：Windows Credential Manager 当前用户凭据存储与秘密脱敏。

## 实现

- 新增 `app-core::credentials`，通过 `CredWriteW`、`CredReadW`、`CredDeleteW` 和 `CredFree` 管理当前用户 Generic Credential。
- 凭据使用 `CRED_PERSIST_LOCAL_MACHINE`，可在当前 Windows 用户后续登录会话继续使用，不写入机器级共享秘密。
- 稳定目标格式为 `ZZHDesktopAssistant/agent/<profile-id>`；profile ID 只允许 1-128 个 ASCII 字母、数字、点、横线或下划线，避免目标命名冲突和路径式注入。
- `SecretString` 不实现明文 `Display`，`Debug` 固定输出 `[REDACTED]`；`redact_secret` 可在错误进入日志或界面前替换已知秘密。
- 秘密拒绝空白值、空字符和超过 Windows 2560 字节上限的输入，错误内容不包含原始输入。
- 进程内 `String` 和写入用字节副本由 `zeroize` 在释放时清零。
- `CredReadW` 返回的系统缓冲区由 RAII 守卫管理，合法大小的凭据 blob 在 `CredFree` 前清零。
- 读取不存在的目标返回 `None`；删除不存在的目标返回 `false`，便于清理和重试保持幂等。
- 非 Windows 平台保留明确的“不支持”错误，不模拟明文存储降级。

## 安全复核

- 配置模型仍只保存不可认证的凭据目标引用，不新增 API Key JSON 字段。
- Windows API 错误只报告操作类型和系统错误，不包含秘密值。
- 写入失败、读取数据无效、UTF-8 无效及测试中途失败路径均释放或清理已取得的敏感资源。
- 使用参数化 Windows API 调用，不经过 shell、环境变量或临时文件。
- 本批安全复核未发现 Critical、High 或 Medium 级问题。

## 自动化验证

- `cargo test -p app-core --offline`：38 项通过，1 项隔离凭据测试默认忽略。
- 隔离凭据往返测试在受限执行会话中因没有 Windows 登录会话而被系统拒绝；在真实当前用户会话中显式运行后 1 项通过。
- 隔离测试使用进程号和纳秒时间戳生成专用目标，写入后验证读取、删除、缺失读取和重复删除；RAII 清理守卫确保失败路径也尝试删除。
- `cargo test --workspace --offline`：128 项通过，2 项系统状态测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
- `cargo build --release -p desktop-assistant --offline`：通过。
- `cargo fmt --all -- --check`：通过。

## 界面验证

本批未修改桌面对话或设置界面，不需要人工界面验证，也没有自动启动或控制用户桌面。

## 下一步

进入 P4 Task 6：接入桌面对话界面的智能体选择、输入、流式回答、停止、重试和错误展示，并提供完整的人工界面验证步骤。
