# ZZH 桌面助手

一款面向 Windows 10/11 的轻量桌面助手，将硬件监控、动态桌面形象和 AI 对话入口集中在一个可常驻的透明窗口中。

> 当前项目处于早期开发阶段。公开版本仅支持 Windows x64，安装包尚未进行代码签名，Windows SmartScreen 可能显示安全提示。

<img width="283" height="436" alt="ZZH 桌面助手监控面板" src="https://github.com/user-attachments/assets/2f90f26c-2c79-401f-924b-7e927a6837a4" />

## 功能

- 透明、无边框、可拖动的动态形象，可始终置顶并吸附到桌面任意位置。
- 点击形象展开监控面板，再次收起时恢复展开前的位置。
- 系统托盘常驻，可显示、隐藏、展开、收起、切换置顶、锁定位置或退出。
- 实时显示 CPU、内存、GPU、显存、CPU 温度、GPU 温度和网络上下行速率。
- 展示最近 5 分钟的 CPU、内存及上下行网络趋势，支持 2、5、10 秒采样间隔。
- AI 模型管理支持 OpenAI 兼容的云端 API、本地服务和 Codex CLI。
- 对话支持连续上下文、流式回答、停止、失败重试和模型切换。
- API Key 只写入 Windows Credential Manager，不保存到 JSON 配置或日志。

## 系统要求

- Windows 10 或 Windows 11，x64 架构。
- 完整温度监控需要通过安装包安装 `ZZHSensorService` 和官方 PawnIO 驱动。
- Intel/NVIDIA 温度可优先使用显卡驱动提供的 Level Zero/NVML；AMD GPU 与 CPU Package/Tctl-Tdie 由传感器服务读取。

## 安装

推荐从仓库的 **Releases** 页面下载 `ZZHDesktopAssistant-Setup.exe`，然后按向导安装。安装器会创建桌面和开始菜单快捷方式，并按需安装经过签名校验和 SHA-256 校验的官方 PawnIO 2.2.0。

根目录构建得到的 `desktop-assistant.exe` 可以单独运行基础监控、托盘和 AI 对话功能，但不包含传感器服务与 PawnIO，因此 CPU 或部分 AMD GPU 温度可能不可用。

卸载 ZZH 桌面助手时不会移除 PawnIO，因为其他硬件监控软件也可能使用该共享驱动。

## AI 模型配置

打开“设置 -> AI 管理”添加模型：

- **云端 API**：填写 OpenAI Chat Completions 兼容地址、模型 ID 和可选 API Key。
- **本地服务**：连接 Ollama、LM Studio、vLLM 等提供的 OpenAI 兼容端点。
- **Codex CLI**：填写 `codex.exe` 或完整路径，附加参数每行一项。

当前 CLI 连接器仅对 Codex CLI 的输出协议做了适配。Hermes、千问、豆包、DeepSeek 等可通过其 OpenAI 兼容 API 接入；它们各自的 CLI 还不能直接作为 Codex CLI 的替代品。

配置文件位于：

```text
%LOCALAPPDATA%\ZZH Desktop Assistant\settings.json
```

配置文件不含 API Key。首次升级时，如果新目录尚无配置，程序会从旧的 `%LOCALAPPDATA%\Xiaoxi Desktop Assistant\settings.json` 复制现有配置；旧文件会保留以便回滚。对话历史当前只保存在进程内存中，退出应用后不会保留。

## 从源码运行

### 开发环境

- Rust stable，最低版本以 [`rust-toolchain.toml`](rust-toolchain.toml) 和 `Cargo.toml` 为准。
- Visual Studio 2022 Build Tools：安装“使用 C++ 的桌面开发”和 Windows 10/11 SDK。
- .NET 8 SDK。
- PowerShell 5.1 或更高版本。

首次构建需要联网下载 Rust crates 和 NuGet 包。

```powershell
cargo run --package desktop-assistant
```

### 质量检查

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
dotnet test services\sensor-service.tests\ZZHSensorService.Tests.csproj --configuration Release
```

### 构建完整安装包

仓库中的 `installer/PawnIO_setup_2.2.0.exe` 是完整 Bundle 的固定输入。构建脚本会先验证其 Authenticode 签名和 SHA-256，再生成 MSI 与安装引导 EXE。

```powershell
powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
```

输出：

```text
desktop-assistant.exe
ZZHDesktopAssistant-Setup.exe
```

这两个文件属于构建产物，已被 `.gitignore` 排除。正式版本应通过 GitHub Releases 发布，不应直接提交到源码历史。

## 项目结构

```text
apps/desktop/                 Slint 窗口、托盘及应用入口
crates/agent-core/            AI 请求、事件、能力和取消协议
crates/agent-http/            OpenAI 兼容 HTTP 连接器
crates/agent-cli/             Codex CLI 子进程连接器
crates/app-core/              配置、模型资料与凭据引用
crates/system-monitor/        Windows 系统指标采样
services/sensor-service/      高精度温度传感器 Windows 服务
installer/                    WiX MSI/Bundle、PawnIO 与第三方声明
xingxiang/                    动态形象素材
docs/                         架构决策、设计计划、验证与发布记录
```

## 已知限制

- 仅支持 Windows x64。
- 安装器与应用当前未进行商业代码签名，可能触发 SmartScreen。
- 不同主板、CPU 和驱动暴露的传感器能力不同；读不到可信的 CPU Package/Tctl-Tdie 或 GPU Core/Edge 时会显示“暂不可用”，不会回退到 ACPI 或 Hot Spot。
- 当前不支持文件拖拽给 AI，也不保存跨进程对话历史。
- 当前 CLI 连接器仅支持 Codex CLI。

## 路线图

- 文件拖拽与附件处理。
- 更多 CLI 智能体适配器。
- 对话历史持久化及会话管理。
- 更多可切换动态形象。
- 安装包和可执行文件代码签名。

## 贡献与安全

提交代码前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。安全问题请不要创建公开 Issue，处理方式见 [SECURITY.md](SECURITY.md)。

第三方组件及其许可证见 [`installer/ThirdPartyNotices.txt`](installer/ThirdPartyNotices.txt)。

## 许可证

本项目以 [MIT License](LICENSE) 开源。
