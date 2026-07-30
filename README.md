# 晓曦桌面助手

面向 Windows 10/11 的低资源桌面监控与智能体入口。当前处于 P2 桌面外壳阶段，正在把技术验证窗口完善为可长期使用的桌面应用。

## 当前可用

- 200 x 200 透明、无边框、默认置顶的晓曦收起态。
- 点击晓曦展开为固定 412 x 640 面板，通过右上角关闭按钮或 ESC 收起。
- 按住晓曦或展开面板顶部可拖动窗口，拖动不会误触发收展。
- 设置页可实时切换始终置顶和位置锁定；锁定后仍可点击晓曦收展。
- 窗口位置、始终置顶和位置锁定自动保存到当前用户的本地配置，并在重启后恢复。
- 窗口始终至少保留 32 个物理像素在当前屏幕内。
- 监控、对话、设置三个基础页签。
- CPU、物理内存、GPU、显存和非回环网卡上下行速率每 2 秒真实刷新。
- GPU 利用率取整机最忙物理引擎；显存显示当前最活跃硬件适配器的总 GPU 内存占用。
- 温度仍处于后续验证阶段，界面明确显示“待接入”。
- 已完成运行时无关的智能体连接器契约、无网络 HTTP 流模拟器和真实子进程 CLI 原型。

本地配置使用同目录临时文件和 Windows 原子替换；损坏配置会保留 `.corrupt` 副本并回退默认值。托盘和单实例属于接下来的 P2 工作。

## 本地运行

```powershell
cargo run -p desktop-assistant
```

质量检查：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

如果受限环境阻止 Rustup 写入用户目录，可直接调用已安装 stable 工具链中的 `cargo.exe`。正常开发终端无需这样处理。

## 工程结构

```text
apps/desktop/       Slint 窗口与应用入口
crates/agent-core/  智能体请求、能力、事件、错误与取消契约
crates/agent-http/  可脚本化的无网络 HTTP 流原型
crates/agent-cli/   使用参数数组和标准输入输出的本地 CLI 原型
crates/app-core/    不依赖 UI 的应用状态
crates/system-monitor/ Windows 原生系统指标采样
docs/               架构、计划、ADR 与验证报告
```
