# P3 监控核心第一批报告

日期：2026-07-31
范围：统一指标时间语义和 5 分钟历史容器。

## 实现

- `MetricSnapshot` 使用 `Instant` 记录一次采样完成时间。
- `SourceStatus` 在已有可用、预热和不支持之外增加暂时错误状态。
- 快照过期规则固定为“年龄严格大于最大允许值时过期”；时间不连续或样本时间在未来时按新鲜处理，避免错误隐藏数据。
- `MetricHistory` 默认保留最近 5 分钟，按时间边界淘汰旧样本，边界时刻的样本继续保留。
- 历史拒绝倒序样本，防止休眠恢复或调用方错误破坏趋势顺序。

## 验证

`cargo test -p system-monitor --offline`：18 项通过。
`cargo test --workspace --offline`：75 项通过，1 项隔离 HKCU 写入测试按设计忽略。
`cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
`cargo build --release -p desktop-assistant --offline`：通过。

新增覆盖：

- Fresh/Stale 精确边界。
- 时间不连续时的保守处理。
- 5 分钟历史边界保留与过期淘汰。
- 倒序样本拒绝和最新样本保持。

## 下一步

把历史容器接入监控 worker，随后生成固定点数的趋势序列；这一批尚未修改 UI，也未增加线程或常驻进程。
