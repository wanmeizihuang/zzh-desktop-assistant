# GitHub 仓库资料

创建或设置仓库时可直接使用以下内容。

## Repository name

```text
zzh-desktop-assistant
```

## Description

```text
轻量 Windows 桌面助手：动态桌面形象、CPU/GPU/内存/温度/网络监控，以及 OpenAI 兼容 API、本地模型和 Codex CLI 对话入口。
```

## Topics

```text
windows desktop-assistant rust slint system-monitor hardware-monitor cpu-temperature gpu-monitor ai-assistant openai-compatible codex-cli system-tray
```

## Website

首个版本可留空；后续可填写项目文档站或 Releases 页面。

## 建议设置

- 默认分支：`main`
- 启用 Issues 和 Discussions。
- 启用 Private vulnerability reporting。
- 合并前要求 `CI / verify` 通过。
- 禁止直接推送 `main`，至少需要一个 Pull Request。
- 启用自动删除已合并分支。
- 发布二进制只使用 Releases，不提交到 Git 历史。

## v0.1.2 Release 标题

```text
ZZH 桌面助手 v0.1.2
```

Release 正文使用 [`docs/releases/v0.1.2.md`](releases/v0.1.2.md)，附件上传：

```text
ZZHDesktopAssistant-Setup.exe
SHA256SUMS.txt
```

发布步骤与验证项目见 [`docs/release-checklist.md`](release-checklist.md)。
