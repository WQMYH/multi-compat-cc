# Qoder CN 官方协议咨询稿

核对日期：2026-10-18（Asia/Shanghai）。本稿可提交给Qoder CN官方支持；未发送，不含本机路径、凭据或私人账号信息。

## 向哪里询问

优先咨询 **Qoder CN**，避免混入国际版账号、区域端点和授权条件：

- CN支持邮箱：**qoder@service.aliyun.com**。该地址见[CN官方文档页脚](https://docs.qoder.cn/cli/authentication)及[CN官网](https://qoder.com.cn/)。
- [Qoder CN问题反馈](https://docs.qoder.cn/qoder/feedback)：从[问题排查页](https://docs.qoder.cn/qoder/troubleshooting)的「问题反馈」进入。该链接已从官方导航确认；本次读取反馈页正文超时，不冒称已经取得表单或提交回执。
- 官方技术社区：使用上述官方文档导航中的「论坛」，请转给Agent SDK／账号认证／模型协议负责人，不假定社区版主能决定授权范围。

## 可直接提交的正文

**主题：咨询Qoder CN账号授权及原始模型API的官方支持边界（非Agent委派）**

您好，我们在基于CC Switch 4.x制作Qoder CN兼容，目标包括：

1. 通过官方机制管理用户明确选定的Qoder CN授权来源、核验账号身份，并区分选择档案、解除本地绑定和供应商退出登录。
2. 允许Codex等兼容客户端将用户选定的Qoder CN账号作为模型供应商。这里需要调用方持有会话历史并执行工具，不是让SDK启动另一个完整Agent接管任务。
3. 为Qoder官方自定义模型入口提供独立BYOK网关，接入其他合法获授供应商。

目前已核对`@qodercn-ai/qodercn-agent-sdk@1.0.53`及其Bundled CLI 1.1.71。源码和合成协议验证不使用真实账号或模型推理。以下是已观察到的边界，而不是断言官方不支持：

- SDK公开了PAT、Service Account及原生CLI认证来源和账号查询能力。
- SDK输入类型中的消息角色是`user`；公开架构由CLI管理上下文、工具执行及Agent循环。现有证据不足以证明可将其作为调用方完全控制的原始模型接口。
- 文档中的`models.read`／`chat.completions`授权示例，以及CN Cloud Agents区域说明，不足以确定CN原始推理URL和完整协议；我们不会将国际版交换地址与CN域名自行拼接。

烦请明确回答，最好提供公开文档或稳定协议链接：

1. **是否正式支持Qoder CN账号向第三方客户端提供原始模型调用？** 如果支持，CN模型目录及推理的Base URL、具体路径、协议类型分别是什么？个人订阅账号、PAT和组织Service Account各自适用吗？
2. **需要什么授权与计费资格？** 请说明官方授权入口、所需scope、有效期／刷新／撤销规则、区域限制，以及第三方客户端使用是否符合相应条款。身份登录或Agent SDK可用是否意味着具有这类模型调用权？
3. **调用方能否提交完整历史并接管工具循环？** 需要保留system／developer／user／assistant消息、工具定义、调用ID及tool result；工具必须由调用方执行，不能被SDK内部执行或变成拒绝结果。支持哪些请求／响应字段和多轮关联方式？
4. **流式成功、失败及中断如何判定？** 请提供SSE事件／终态和错误格式，说明超时、取消及接收未知时可用的查账或幂等机制；我们不会自动重放效果不明的请求。
5. **账号管理有哪些官方可用接口？** SDK／CLI是否提供浏览器登录、退出、账号枚举、显式账号选择及身份失效通知？指定CLI配置目录与桌面账号是否独立？如果桌面账号切换没有第三方公共契约，请明确说明，我们会保留不支持边界而不读写私有缓存。
6. **BYOK客户端有什么稳定协议要求？** 对Chat Completions／Responses，是否有推荐的模型目录、完整历史、工具字段、store／stream及终态要求？是否存在可用于兼容验证的公开样例或测试模式？

如果仅支持完整Agent SDK或Cloud Agents而不支持第1—4项的原始模型能力，请直接确认其边界。我们会将该接入方向保留为受阻，不用Agent委派冒称已经完成模型兼容；其他账号档案与BYOK方向继续独立推进。

谢谢。

## 随询问附带的公开依据

- [CN SDK认证](https://docs.qoder.cn/en/cli/sdk/authentication)
- [CN SDK工作原理](https://docs.qoder.cn/en/cli/sdk/how-it-works)
- [CN TypeScript Reference](https://docs.qoder.cn/cli/sdk/references-typescript)
- [CN CLI登录与认证](https://docs.qoder.cn/cli/authentication)
- [CN Cloud Agents Quickstart](https://docs.qoder.cn/en/cloud-agents/quickstart)
- [Qoder自定义模型](https://docs.qoder.cn/qoder/custom-models)

收到答复时分别登记「官方明确支持」「官方明确不支持」「尚未回答」，绑定所述区域、账号类别及协议。支持答复仍不代替源码实现、隔离回归、安装和指定客户端实机验收。
