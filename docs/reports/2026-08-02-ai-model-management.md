# AI 模型管理实现报告

日期：2026-08-02
范围：设置页 AI 管理、模型配置迁移、凭据事务和对话模型目录刷新。

## 交付结果

- 设置页新增“AI 管理”入口和窗口内模态层，可新增、编辑、检查和删除模型。
- 支持云端 OpenAI 兼容 API、本地 Ollama/LM Studio/vLLM 类兼容服务和 Codex CLI。
- 对话页只显示已保存模型，不再隐式注入配置之外的 Codex 条目。
- 首次升级且模型为空时，可将检测到的 Codex 转为可编辑、可删除的普通模型；初始化只执行一次。
- HTTP 模型记录云端/本地部署位置，本地服务在连接器能力中声明为本地执行。
- 多个 Codex 配置使用各自稳定 ID 和显示名称，不会在连接器目录中冲突。
- 新增、编辑和删除成功后立即刷新对话模型列表，同时保留对话历史和请求控制器。
- 回答进行中 AI 管理保持可查看，新增、保存和删除操作禁用。

## 凭据与一致性

- API Key 只进入 Windows Credential Manager，JSON 只保存稳定凭据引用。
- 编辑时不回填旧密钥；留空保留，输入新值替换，独立状态用于清除。
- 配置保存使用带确认的即时原子写入，并与已有延迟设置保存串行执行。
- 配置保存失败时恢复旧凭据；删除模型失败时不更新内存目录，不显示假成功。
- 附加 CLI 参数按“一行一项”保存并作为参数数组执行，不经过 shell 解析。

## 自动化验证

- `cargo test -p app-core --offline`：43 项通过，1 项当前用户凭据写入测试按设计忽略。
- `cargo test -p agent-cli --offline`：12 项通过。
- `cargo test -p desktop-assistant --offline`：41 项通过，1 项当前用户启动项写入测试按设计忽略。
- `cargo test --workspace --offline`：156 项通过，2 项系统状态写入测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过，零警告。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。
- `cargo build --release -p desktop-assistant --offline`：通过；根目录 EXE 已更新为 18,973,696 字节。
- 自动化覆盖配置校验、旧配置迁移、稳定 ID、凭据替换回滚、删除回退、一次性 Codex 初始化、目录重载保留控制器和同步配置确认。

根目录与 Release 产物 SHA-256 均为 `1F2C9DC62AE6DB331BA67939B65D9AD8E49ECD98BC3D8C9840F868C90516EF47`。

开发前备份保存在 `backups/current-pre-ai-manager-20260802.zip`，SHA-256 为 `E7B72CBC5912037CE4EABEE2D55FD3F939982142AD6F7E41B95102C63FB54BE0`；旧备份已按“只保留一份”约定删除。

## 人工确认范围

按用户确定的验证方式，本批不自动启动、点击、截图或控制桌面程序。AI 管理弹窗布局、中文输入、密钥掩码、增删改流程、忙碌期只读和对话模型选择由用户按交付清单人工确认。

## 后续边界

- 本批不直接加载 `.gguf` 等模型文件。
- 通用 CLI 协议、厂商预置模板、文件拖放和上传确认仍属于后续 P5 工作。
- “检查配置”不发送网络请求；真实端点、认证和模型 ID 在首次对话时由连接器验证。
