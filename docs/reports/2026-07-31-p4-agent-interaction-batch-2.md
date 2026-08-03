# P4 智能体交互第二批报告

日期：2026-07-31
范围：监控界面调整、连接器目录与非密钥连接配置。

## 前置界面调整

- 最近 5 分钟趋势由离散点阵改为相邻有效样本线段。
- CPU 与内存使用单曲线，网络接收与发送使用共享量程双曲线。
- 任一侧样本缺失时不生成对应线段，避免跨越缺失区间。
- 实机截图确认 Slint `Path` 默认 `contain` 会把正方形 viewbox 居中压缩；已改为 `fill`，让曲线铺满宽而矮的完整趋势区域。
- 网络卡片与趋势图例将方向箭头独立绘制：下行使用蓝色，上行使用红色；速率和标签文字保持原有中性色。
- 默认翡翠形象从 512px 原始帧先高质量缩至 192px，再以双线性方式烘焙为 128px 精灵帧，增加透明边缘过渡且不提高运行时解码尺寸；备选形象资源保持不变。
- 展开监控页支持 2、5、10 秒采样频率，默认 2 秒。
- 设置切换会立即更新监控线程并持久化；后台可见和托盘隐藏仍保持 10、20 秒周期。

## P4 Task 2

- `ConnectorCatalog` 按注册顺序保存少量 `Arc<dyn AgentConnector>`。
- 首个连接器成为默认选择；目录支持稳定 ID 查询、显式选择和缺失错误。
- 空 ID 与重复 ID 会在目录变更前被拒绝。
- 配置版本升级到 4，新增 HTTP endpoint/model 和 CLI executable/arguments 模型。
- 凭据只保存 Credential Manager target 引用，不提供 API Key 明文字段。
- 旧版本配置默认迁移为空连接器配置，悬空的已选 profile 会被清除。

## 自动化验证

- `cargo test --workspace --offline`：108 项通过，1 项隔离 HKCU 测试按设计忽略。
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过。
- `cargo build --release -p desktop-assistant --offline`：通过。
- `git diff --check`：通过，仅提示现有 Windows 行尾转换信息。

Release 构建位于 `target/release/desktop-assistant.exe`。根目录旧版进程仍在运行，因此本批未自动结束进程或覆盖根目录 `desktop-assistant.exe`。

## 人工界面验证

1. 通过托盘菜单退出当前运行的旧版助手，再启动 `target/release/desktop-assistant.exe`。
2. 展开监控页，等待至少两个采样周期。预期 CPU、内存显示为横向铺满趋势区域的连续线段，网络显示铺满趋势区域的收发双曲线，不再显示居中的短线或独立点阵。
3. 打开设置页，在“采样频率”中依次选择 5 秒、10 秒和 2 秒。预期选中项使用蓝色高亮，监控页顶部说明同步显示所选秒数。
4. 选择 5 秒后退出并重新启动助手。预期设置页仍选中 5 秒，监控页约每 5 秒更新一次。
5. 恢复选择 2 秒。预期监控页约每 2 秒更新一次，并在再次启动后保持该选择。

本批没有自动启动、点击、截图或控制桌面界面。

## 下一步

进入 P4 Task 3：实现 OpenAI 兼容 HTTP 的真实 SSE 流式传输与本机模拟服务测试。
