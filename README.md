# 晓曦桌面助手

面向 Windows 10/11 的低资源桌面监控与智能体入口。当前处于 P1 技术验证阶段，已完成第一个可运行窗口原型。

## 当前可用

- 200 x 200 透明、无边框、始终置顶的晓曦收起态。
- 点击晓曦展开为固定 412 x 640 面板，再通过右上角关闭按钮收起。
- 监控、对话、设置三个基础页签。
- 监控页当前使用明确标注的原型数据，尚未接入真实系统指标。

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
crates/app-core/    不依赖 UI 的应用状态
docs/               架构、计划、ADR 与验证报告
```

