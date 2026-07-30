# P2 桌面外壳第一批验证报告

## 范围

- 应用生命周期：`Starting`、`Collapsed`、`Expanded`、`Hidden`、`Exiting`。
- 桌面行为：ESC 收起、始终置顶开关、位置锁定。
- 拖动策略：锁定后不启动原生拖动，点击收展仍可使用。

## 自动化结果

| 检查 | 结果 |
|---|---|
| `cargo test --workspace --offline` | 46 个测试通过 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo build -p desktop-assistant --release --offline` | 通过 |
| `git diff --check` | 通过 |

## Windows 实机结果

环境为 Windows 11 x64、150% DPI。

| 场景 | 结果 |
|---|---|
| 收起态尺寸 | 200 x 200 |
| 展开态尺寸 | 412 x 640 |
| 设置页布局 | 无重叠或文本截断 |
| 置顶关闭/恢复 | Windows 顶层标志正确切换并恢复 |
| 锁定后拖动 | 窗口坐标不变 |
| ESC 收起 | 恢复为 200 x 200 |
| 收起态工作集 | 30,625,792 bytes（约 29.2 MiB） |
| 收起态私有内存 | 13,258,752 bytes（约 12.6 MiB） |

## 已知限制

- 设置尚未落盘，重启后恢复默认值。
- `Hidden` 和 `Exiting` 已有核心状态，但要等托盘入口接入后完成端到端验证。
- P1 的 24 小时内存趋势和 ADR-002 最终闸门仍处于延期状态。
