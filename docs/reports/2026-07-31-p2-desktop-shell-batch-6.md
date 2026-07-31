# P2 桌面外壳第六批验证报告

> 后续修正：本报告当时只检查了恢复后的原生窗口可见标志，没有继续观察进程生命周期。用户复测发现显示路径会触发 Release panic；根因和修复见 `2026-07-31-p2-desktop-shell-batch-7.md`。

## 范围

- 主窗口在启动和托盘恢复后均不显示任务栏应用按钮。
- 托盘右键隐藏助手后，可通过同一菜单再次显示助手。
- 展开前记录收起态位置，面板收起后恢复该位置。

## 实现

- 窗口显示后同时调用 Winit 的跳过任务栏接口，并在 Win32 扩展样式中清除 `WS_EX_APPWINDOW`、添加 `WS_EX_TOOLWINDOW`。
- 隐藏和显示同时更新 Slint 与原生 Winit 窗口可见性；每次显示后延迟重申工具窗口样式。
- 收起态物理坐标作为一次展开周期内的临时锚点保存在 `WindowState`。收起尺寸生效后恢复锚点并持久化最终坐标，隐藏/显示不会消费该锚点。

## 自动化结果

| 检查 | 结果 |
|---|---|
| `cargo test --workspace --offline` | 68 项通过，1 项隔离 HKCU 测试按设计默认忽略 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo build --release -p desktop-assistant --offline` | 通过 |
| `git diff --check` | 通过 |
| 展开周期位置锚点单元测试 | 通过，锚点仅恢复一次 |

## Windows 交互结果

环境为 Windows 11 x64、150% DPI。

| 场景 | 结果 |
|---|---|
| 主窗口扩展样式 | `WS_EX_TOOLWINDOW = true`，`WS_EX_APPWINDOW = false` |
| 托盘隐藏前 | 主窗口可见 |
| 托盘选择“隐藏助手”后 | 主窗口不可见 |
| 托盘选择“显示助手”后 | 主窗口恢复可见，工具窗口样式保持不变 |
| 边缘收起位置 | DPI 虚拟化探针坐标 `(1659, 1051)` |
| 展开位置与尺寸 | `(1268, 432)`，`412 x 640`，面板完整位于工作区内 |
| 再次收起位置与尺寸 | 恢复为 `(1659, 1051)`，`200 x 200` |
| 测试清理 | 已恢复测试前窗口位置，修复版继续运行 |

原有唯一备份 `backups/p2-task5-3c72131.zip` 和标签 `backup/p2-task5-20260730` 保持不变。
