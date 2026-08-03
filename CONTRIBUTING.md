# 贡献指南

感谢你参与 ZZH 桌面助手。提交 Issue 或 Pull Request 前，请先搜索是否已有相同问题。

## 开发准备

项目仅在 Windows x64 上运行和验证。开发机需要 Rust stable、Visual Studio 2022 Build Tools（C++ 桌面开发与 Windows SDK）和 .NET 8 SDK。

```powershell
git clone <repository-url>
cd <repository-directory>
cargo build --workspace
dotnet build services\sensor-service.tests\ZZHSensorService.Tests.csproj
```

## 分支与提交

- 从最新主分支创建短生命周期分支。
- 一个 Pull Request 聚焦一个问题，避免夹带无关格式化或重构。
- 提交信息使用简短祈使句，说明“做了什么”。
- 不要提交 API Key、凭据、本机配置、日志、备份或构建产物。

推荐提交前执行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
dotnet test services\sensor-service.tests\ZZHSensorService.Tests.csproj --configuration Release
```

涉及界面、托盘、窗口位置或硬件传感器的变更，还应在 Windows 10/11 实机人工验证，并在 Pull Request 中写明硬件与验证结果。

## Pull Request 要求

- 清楚说明问题、解决方式和影响范围。
- 新增或修改行为时同步测试和相关文档。
- UI 变更附截图或录屏。
- 安装器变更说明升级、卸载、快捷方式与管理员权限验证结果。
- 第三方依赖变更说明许可证和分发方式。

## 报告问题

功能缺陷和需求请使用仓库 Issue 模板。漏洞或敏感问题请按 [SECURITY.md](SECURITY.md) 私下报告。
