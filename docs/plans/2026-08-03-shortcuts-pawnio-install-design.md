# 快捷方式与 PawnIO 依赖修复设计

## 决策

主 MSI 负责安装应用、按需温度服务、桌面快捷方式与开始菜单快捷方式。快捷方式均指向 Program Files 中的同一桌面程序，并使用安装包内的应用图标；开始菜单目录通过独立组件和注册表键路径管理，卸载时一并清理。

CPU Package/Tctl-Tdie 继续由 LibreHardwareMonitor 0.9.6 读取，不恢复 ACPI。Intel CPU 的 MSR 温度读取依赖 PawnIO，因此发布物新增 WiX Burn 安装引导程序。引导程序原样携带官方签名的 PawnIO 2.2.0 安装器，构建时强制校验固定 SHA-256 与 Authenticode 签名；目标电脑未安装 PawnIO 时，以官方 `-install -silent` 参数先安装共享驱动，再安装主 MSI。PawnIO 可能被其他监控软件共享，卸载助手时不自动删除。

温度服务启动后先检测 PawnIO 服务注册项和设备节点。缺少驱动、设备不可用、传感器初始化失败和采样失败分别通过只读命名管道返回机器可读错误码。桌面端将这些状态转换为明确提示，不再把依赖缺失显示成“当前硬件不支持”。CPU 仍仅接受 CPU Package/Tctl-Tdie，GPU 仍仅接受 GPU Core/Edge。

## 验证

自动验证包括 Rust 与 .NET 单元测试、发布构建、WiX MSI/Bundle 构建、PawnIO 哈希与签名校验。安装后的快捷方式、UAC、必要时重启以及不同硬件上的实时温度由用户按交付步骤人工确认。
