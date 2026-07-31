# P2 桌面外壳第七批回归报告

## 问题

第六批验证只确认托盘恢复后原生窗口短暂变为可见，没有继续检查进程存活。用户实际复测时，选择“显示助手”后没有看到窗口，随后桌面助手进程退出。

Windows 应用日志确认最终发布进程 `22852` 在显示路径中以 `0xc0000409` 异常退出。使用 Release PDB 解析故障偏移 `0x335795`，调用点落在 `std::process::abort`，与工作区 Release 配置 `panic = "abort"` 一致。

## 根因与修复

`restore_window` 持有 `WindowState` 的 `RefCell` 可变借用时调用 `ui.set_expanded()`。该窗口调用可能同步派发 `Resized` 事件，事件处理器再次借用同一状态并触发 panic；Release 配置直接中止进程。

- 所有窗口展开、收起、隐藏和恢复操作均先完成状态转换并释放借用，再调用 Slint/Winit API。
- 托盘状态同步先复制阶段、置顶和锁定值，释放状态借用后再更新托盘属性。
- Slint 成为窗口可见性的唯一管理层，不再额外直接调用 Winit `set_visible()`。
- 每次显示和延迟应用 Win32 工具窗口样式后显式请求重绘。

## 自动化结果

| 检查 | 结果 |
|---|---|
| `cargo test --workspace --offline` | 68 项通过，1 项隔离 HKCU 测试按设计默认忽略 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo build --release -p desktop-assistant --offline` | 通过 |
| `git diff --check` | 通过 |

## Windows 回归结果

环境为 Windows 11 x64、150% DPI，使用正常 `panic = "abort"` Release 构建。

连续执行三轮真实托盘菜单操作，每轮隐藏 5 秒后再显示：

| 轮次 | 隐藏后可见 | 显示后可见 | 进程存活 |
|---|---|---|---|
| 1 | 否 | 是 | 是 |
| 2 | 否 | 是 | 是 |
| 3 | 否 | 是 | 是 |

测试结束后 `desktop-assistant.exe` 保持响应，新增 Windows 应用崩溃事件为 0；`WS_EX_TOOLWINDOW` 保持启用，`WS_EX_APPWINDOW` 保持禁用。最终修复版进程继续运行。
