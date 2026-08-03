# GPU 温度选择修复报告

日期：2026-08-02
范围：GPU 类别识别、Intel 温度读取、降级文案与当前设备探测。

## 结果

- 温度卡现在只表示 GPU 温度，不再读取或显示 ACPI 热区。
- 通过 DXCore 的 `IsHardware` 与 `IsIntegrated` 属性识别物理显卡，避免用少量专用显存误判 Intel Iris Xe。
- 系统存在独显时只选择独显；没有独显时只选择核显。目标类别确定后不允许回退到另一块显卡。
- Intel 目标通过 System32 中的 `ze_loader.dll` 使用 Level Zero Sysman，兼容新旧初始化入口，不新增常驻进程或采样线程。
- 同一目标设备存在多个传感器时依次选择 GPU、GPU Board、Global；只接受 0 至 150 摄氏度的有限数值。
- 驱动未提供目标温度传感器时显示“核显/独显不支持温度读取”，临时读取失败则提示自动重试。

## 当前设备

本机只检测到 `Intel(R) Iris(R) Xe Graphics`，分类为核显。Intel Level Zero 可用，但驱动枚举到 0 个温度传感器，因此程序应显示：

- 卡片标题：`GPU 温度`
- 数值：`不支持`
- 详情：`核显不支持温度读取`

旧版约 28 摄氏度的数值来自 ACPI 热区，不应再出现。

## 自动化验证

- `cargo test -p system-monitor --offline`：31 项通过。
- `cargo test -p desktop-assistant --offline`：34 项通过，1 项系统状态写入测试按设计忽略。
- `cargo test --workspace --offline`：142 项通过，2 项系统状态写入测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过，零警告。
- 当前设备探针连续三次返回 `temperature_target: Some(Integrated)` 和 `temperature_celsius.status: Unavailable`。

## 后续边界

NVIDIA NVML、AMD 原生温度接口和 NVIDIA/AMD/Intel 多设备兼容矩阵仍属于 P6。本批只完成 Intel 原生路径和统一选择策略，不宣称其他厂商温度读取已经支持。

界面操作和显示内容按用户决定由人工确认，本批未自动启动、点击、截图或控制桌面程序。
