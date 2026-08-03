# 发布清单

## 1. 确认版本

- `Cargo.toml` 的 `workspace.package.version` 与 `Cargo.lock` 一致。
- `installer/Package.wxs` 与 `installer/Bundle.wxs` 使用相同版本。
- `CHANGELOG.md` 和对应的 `docs/releases/vX.Y.Z.md` 已更新。

## 2. 检查仓库

```powershell
git status --short
git diff --check
git ls-files | Select-String -Pattern '\.(exe|msi|pdb|ilk)$'
```

确认没有凭据、本机路径、备份、日志和意外构建产物。`installer/PawnIO_setup_2.2.0.exe` 是经许可原样再分发的固定构建输入，不属于应用构建产物。

## 3. 自动检查

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
dotnet test services\sensor-service.tests\ZZHSensorService.Tests.csproj --configuration Release
```

## 4. 构建安装包

```powershell
powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
```

构建脚本会验证 PawnIO 安装器签名与固定 SHA-256，并生成根目录的 `ZZHDesktopAssistant-Setup.exe`。

## 5. 实机验证

至少在一台干净的 Windows 10/11 x64 设备验证：

- 全新安装、覆盖升级和卸载。
- 桌面与开始菜单快捷方式。
- 托盘显示/隐藏、展开/收起、置顶、锁定和退出。
- 单实例恢复、开机启动和多显示器边缘位置。
- CPU/GPU/内存/显存/网络指标与温度读取。
- OpenAI 兼容 API、本地服务和 Codex CLI 对话。
- 卸载后 PawnIO 保留，应用和传感器服务已移除。

## 6. 生成校验文件

```powershell
Get-FileHash ZZHDesktopAssistant-Setup.exe -Algorithm SHA256 |
    ForEach-Object { "$($_.Hash.ToLower())  $([IO.Path]::GetFileName($_.Path))" } |
    Set-Content SHA256SUMS.txt -Encoding ascii
```

## 7. 创建 GitHub Release

- 创建带注释标签 `vX.Y.Z`。
- Release 标题使用 `ZZH 桌面助手 vX.Y.Z`。
- 正文使用对应的 `docs/releases/vX.Y.Z.md`。
- 上传 `ZZHDesktopAssistant-Setup.exe` 和 `SHA256SUMS.txt`。
- 发布后从 GitHub 下载附件，重新核对 SHA-256 并执行一次安装冒烟测试。
