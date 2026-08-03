# P1 智能体连接器原型验证报告

日期：2026-07-30
范围：统一契约、无网络 HTTP 模拟流、真实测试子进程 CLI 流

## 已实现

- `agent-core` 定义描述信息、能力、消息、附件元数据、流式事件、机器可读错误和连接器 trait。
- `AgentConnector::start` 返回运行句柄，通过标准库 MPSC 通道输出 `Started`、`TextDelta`、`Completed`、`Cancelled` 或 `Failed`。
- 取消令牌基于 `Arc<AtomicBool>`；显式取消与 `AgentRun` 离开作用域都会通知工作线程。
- 请求在后台工作开始前校验空提示、附件模式和图片能力，不读取附件内容。
- `agent-http` 提供可脚本化成功分片、401、429、503、断流与取消场景，不访问网络。
- `agent-cli` 使用可执行文件路径和参数数组启动子进程，提示词经 stdin 传入，stdout 逐行转为文本事件。
- Windows CLI 子进程使用 `CREATE_NO_WINDOW`；取消执行 kill + wait，stderr 最多保留 8 KiB。

## 验证结果

全工作区共 41 项测试：

| 模块 | 测试数 | 重点 |
|---|---:|---|
| `agent-core` | 7 | 请求校验、能力拒绝、错误语义、事件、显式/自动取消 |
| `agent-http` | 4 | 成功分片、状态码映射、断流、取消 |
| `agent-cli` | 4 | 真实进程输出、字面元字符、退出码、进程终止 |
| 现有窗口与监控 | 26 | 原有回归测试 |

格式检查、全 workspace 测试和严格 Clippy 均通过。

HTTP 演示输出：

```text
Started { request_id: RequestId(1) }
TextDelta { request_id: RequestId(1), text: "Hello" }
TextDelta { request_id: RequestId(1), text: " from" }
TextDelta { request_id: RequestId(1), text: " Mock HTTP" }
Completed { request_id: RequestId(1) }
```

CLI 演示输出：

```text
Started { request_id: RequestId(1) }
TextDelta { request_id: RequestId(1), text: "received:hello from CLI" }
TextDelta { request_id: RequestId(1), text: "done" }
Completed { request_id: RequestId(1) }
```

测试提示词 `hello & echo injected` 通过 stdin 被测试子进程原样接收，没有 shell 解释。退出码 7 和受限 stderr 被转为 `Process` 错误；长时间运行的夹具在取消后终止并产生 `Cancelled`。

## 资源与安全结论

三个连接器 crate 尚未链接到桌面应用，当前常驻资源基线不变。连接器只在调用时创建有限生命周期线程，CLI 子进程只按需启动。本阶段不使用网络、不需要 API Key、不访问 Windows Credential Manager，也不读取或上传文件。

核心契约不依赖 Tokio 或特定 HTTP 库。P4 可在 HTTP 实现内部引入异步运行时，而无需改变 Slint UI 消费的事件模型。

## 已知限制

- HTTP 原型只验证事件和错误映射，没有验证 SSE、JSON、TLS、代理、401/429 响应体或真实断流恢复。
- CLI 当前只把最后一条用户消息写入 stdin，尚未序列化完整对话历史、工作目录、环境变量或厂商参数。
- 标准库 MPSC 为无界通道；P4 必须增加响应长度限制或有界背压，防止异常上游无限输出。
- 尚未实现超时定时器、重试调度、凭据、日志脱敏和 UI 侧停止/重试状态机。
- 附件能力只做前置拒绝，文件安全确认与实际传递属于 P5。

## 结论

HTTP 类与 CLI 类智能体能够共享同一请求、事件、错误和取消契约，ADR-0003 的核心假设通过 P1 原型验证。下一项 P1 工作是冷启动与 24 小时资源趋势采集，然后完成 ADR-002 技术栈闸门决策。

