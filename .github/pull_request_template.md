## 变更说明

<!-- 说明问题、解决方式和影响范围。 -->

## 验证

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `dotnet test services\sensor-service.tests\ZZHSensorService.Tests.csproj --configuration Release`
- [ ] 已完成适用的 Windows 实机人工验证，或在下方说明未验证原因

人工验证环境与结果：

## 截图或录屏

<!-- UI 变更必须提供；其他变更可删除本节。 -->

## 检查清单

- [ ] 变更范围聚焦，不包含无关重构或生成文件
- [ ] 新增或修改行为已有测试或验证说明
- [ ] 已同步相关文档
- [ ] 不包含凭据、个人信息、本机配置、日志或构建产物
- [ ] 第三方依赖变更已检查许可证与分发要求
