# ADR-0006：使用按需提权传感器服务提供精确温度

## 状态

已接受（2026-08-02）

## 背景

Windows ACPI 热区不能证明是 CPU Package，现有 Intel Level Zero 也不能覆盖全部 GPU 厂商。项目要求温度准确、主程序保持普通用户权限，并在 NVIDIA、AMD、Intel 和无独显设备上诚实降级。

## 决策

完全删除 ACPI CPU 温度。安装一个手动启动的 `ZZHSensorService`，由 LibreHardwareMonitor 读取 CPU Package/Tctl-Tdie，并作为 GPU Core/Edge 的同厂商回退源。桌面端仍负责显卡目标选择和可用的厂商原生读取。服务通过只出不进、带 ACL 和大小限制的本机命名管道发布只读快照；30 秒无客户端后停止。

发布使用 WiX 4.0.6 MSI。服务为 .NET 8 x64 自包含单文件，固定 LibreHardwareMonitorLib 0.9.6，并履行 MPL-2.0 再分发义务。

## 影响

### 正面

- CPU 和 GPU 都能映射到明确传感器语义。
- 主桌面程序不需要管理员权限。
- 不依赖目标电脑预装 .NET、HWiNFO 或 LibreHardwareMonitor GUI。
- 温度服务失败不会中断 CPU、内存、网络等基础监控。

### 负面

- 交付从单 EXE 变为 MSI，安装时需要管理员权限。
- 增加托管服务的磁盘和内存开销。
- 某些 Windows 内核驱动策略仍可能阻止 CPU 传感器读取。
- 需要维护服务协议、安装升级和第三方许可证。

### 中性

- 旧版绿色 EXE 仍可运行，但没有精确 CPU 温度服务。
- 历史 ACPI 文档保留为阶段记录，不再代表当前实现。

## 备选方案

- 继续 ACPI：占用最低，但数据语义不可靠，已拒绝。
- 要求 HWiNFO 常驻：准确但引入外部安装和许可依赖，已拒绝。
- 每次启动临时提权辅助进程：无需服务安装，但反复 UAC，已拒绝。
- 主程序整体提权：扩大攻击面并影响拖放，已拒绝。

## 参考

- LibreHardwareMonitor 0.9.6：`https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases/tag/v0.9.6`
- MPL-2.0：`https://www.mozilla.org/MPL/2.0/`
- 详细设计：`docs/plans/2026-08-02-accurate-temperature-engine-design.md`
