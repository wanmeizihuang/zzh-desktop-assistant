# 更新日志

本项目的重要变更记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 变更

- AI 模型与桌面设置目录更名为 `%LOCALAPPDATA%\ZZH Desktop Assistant`，并自动复制旧目录中的现有配置。

### 计划

- 文件拖拽与附件处理。
- 更多本地 CLI 智能体适配器。
- 持久化对话会话。

## [0.1.2] - 2026-08-03

### 新增

- 可拖动、置顶、锁定和隐藏的动态桌面形象。
- 系统托盘菜单、单实例恢复、开机启动和窗口位置持久化。
- CPU、内存、GPU、显存、CPU/GPU 温度和网络上下行监控。
- 2、5、10 秒采样与最近 5 分钟趋势曲线。
- 基于 Windows 服务、LibreHardwareMonitor、PawnIO、Level Zero 和 NVML 的混合温度采集。
- OpenAI 兼容云端/本地 API 与 Codex CLI 模型管理和流式对话。
- Windows Credential Manager API Key 存储。
- WiX 安装器、桌面快捷方式和开始菜单快捷方式。

### 修复

- 修复托盘隐藏后无法恢复助手的问题。
- 修复展开窗口超出当前显示器工作区的问题。
- 修复温度服务短暂读取失败导致数值频繁闪烁为不可用的问题。
