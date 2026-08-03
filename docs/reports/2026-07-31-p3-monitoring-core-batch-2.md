# P3 监控核心第二批报告

日期：2026-07-31
范围：历史接入监控 worker、容量上限和趋势降采样。

## 实现

- 现有 `system-monitor` worker 每次采样后写入同线程内的 `MetricHistory`。
- 历史继续按 5 分钟时间窗口淘汰，同时默认最多保留 301 个样本。
- 倒序样本不会进入历史或发送给 UI。
- `trend_points` 可生成固定点数序列；数据充足时保留首尾，只有一个点时选择最新样本。
- 趋势点保留原始单调时间和 `Option` 值，后续 UI 可以明确显示缺失区间。

## 验证

`cargo test -p system-monitor -p desktop-assistant --offline`：34 项通过，1 项隔离 HKCU 测试按设计忽略。
`cargo test -p system-monitor --offline`：23 项通过。
`cargo test --workspace --offline`：80 项通过，1 项隔离 HKCU 测试按设计忽略。
`cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
`cargo build --release -p desktop-assistant --offline`：通过。

新增覆盖：

- 空历史读取。
- 相同时间戳高频写入时的最大容量。
- 无点数预算、单点预算和多点降采样。
- 趋势首尾保留与中间缺失值保留。

## 下一步

进入 Task 3，完善计数器异常间隔、网卡切换和暂时错误恢复；之后将趋势序列接入仅监控页可见时重绘的 UI 模型。
