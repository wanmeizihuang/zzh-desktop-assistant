# GPU 温度选择设计

日期：2026-08-02
状态：已确认

## 问题

当前温度卡读取 Windows `Thermal Zone Information` 中最高的 ACPI 热区。该热区可能代表机身、主板或固件聚合区域，无法映射到具体 GPU；本机显示约 28°C 的数值不是可验证的 Intel Iris Xe 温度。

## 决策

温度卡改为只表示 GPU 温度，并使用以下唯一选择规则：

1. 通过 DXGI 枚举物理 Intel、NVIDIA 和 AMD 适配器，忽略软件与虚拟显示适配器。
2. 只要存在专用显存大于零的独立显卡，就选择独显类别并完全忽略核显温度。
3. 没有独显时选择核显类别。
4. 只允许厂商原生接口为已选类别返回温度；不得回退到另一类别或 ACPI 热区。
5. 驱动未暴露温度传感器时显示“核显/独显不支持温度读取”，不估算、不伪造。

## 首批数据源

首批接入 Intel Level Zero Sysman。动态从 Windows System32 加载 `ze_loader.dll`，不捆绑运行时、不新增进程或线程。兼容新式 `zesInit/zesDriverGet/zesDeviceGet` 和旧式 `zeInit/zeDriverGet/zeDeviceGet + ZES_ENABLE_SYSMAN=1`；设备属性必须与 DXGI 选出的 Intel 核显或 Intel 独显类别一致。优先选择 GPU 温度传感器，其次 GPU Board，最后才接受同一目标设备的 Global 温度。

本机诊断确认 Intel Iris Xe 和 Level Zero 加载器可用，但驱动枚举到 0 个温度传感器，所以预期降级为“核显不支持温度读取”。NVIDIA NVML 和 AMD 原生适配器后续按相同接口加入；在未实现或驱动不支持时，如果系统存在独显，仍只显示独显不支持，不得读取 Intel 核显或 ACPI 数值。

## 状态与验证

`MetricSnapshot` 增加温度目标类别，使无数值状态仍能解释为核显或独显。纯逻辑测试覆盖独显优先、无独显回退核显、虚拟适配器忽略、Intel 设备类别匹配和传感器优先级。Windows 实际接口只做只读探测；界面数值和文案由用户人工确认。
