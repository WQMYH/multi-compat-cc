# Qoder CN Chat Completions 本机观察夹具

以下命令从仓库根目录执行。当前证据、任务分工和未决项见 [兼容开发交接](../../docs/qoder-cn-compatibility-handoff.md)。

仅用于 Qoder CN 独立桌面官方自定义模型入口的隔离验证前置。没有真实上游、代理转发、客户端注册、生产 CC Switch 修改或真实模型调用。Responses / Anthropic 不实现，返回 404，不重放未知请求。

## 离线检查

```powershell
node compat/qoder-cn/check.mjs
```

Node 标准库，无安装步骤。检查自行监听 `127.0.0.1` 的系统分配临时端口，完成/失败都关闭连接和监听。验证文本 JSON、实际分段到达的文本 SSE、一次 JSON 工具 call/result、一次分段 SSE 工具 call/result，以及拒绝与脱敏边界；最后单独启动真实 CLI 子进程，验证启动 PID/目录入口并关闭自己的子进程和端口。离线工具结果是检查程序合成的，不能证明 Qoder 或真实工具执行。

## Root 后续官方 UI 实测

在独立测试 workspace 和新建测试会话中，关闭与真实任务的关联。此包的实现者不操作 Qoder。Root 获准操作后，一次启动：

```powershell
node compat/qoder-cn/fixture.mjs
```

保持该终端开启。启动 JSON 给出实际 Base URL `http://127.0.0.1:<临时端口>/v1`、测试 API Key `qoder-fixture-only` 和三个 Model IDs。在官方自定义模型 UI 选择 OpenAI Chat Completions，填入这些值；若 UI 拒绝 HTTP/loopback，记录可见拒绝即停止该路线，不切换代理或猜配置字段。API Key 是公开的夹具凭据，不是真实密钥。

1. `qoder-fixture-text`：发送纯测试文字。响应固定 `FIXTURE_TEXT_OK`；若客户端请求流式，此模型同样返回合法 SSE。只有观察日志 `stream:false` 和 UI 文本同时成立，才可记为实机非流式文本通过；`stream:true` 属于流式文本结果。
2. `qoder-fixture-sse`：发送纯测试文字。仅接受客户端 `stream:true`，分两段返回 `FIXTURE_SSE_OK`，间隔各 80ms，最后 finish_reason=stop 和 `[DONE]`。日志的 sseFrames/doneSent 证明夹具发送；UI 收到完整标记证明客户端解析；要宣称逐段展示，还需 UI 的可见时序证据。422 表示客户端没有请求流式，不能算 SSE 通过。
3. `qoder-fixture-tool`：默认只会请求客户端实际提供的 `fixture_echo` 工具，参数 `{ "message": "fixture-safe" }`。未提供则返回 `configured_tool_not_offered`，日志留有界工具名/required/property type，属于发现入口，不是工具失败或通过。夹具绝不执行工具，也不自选客户端工具。

真实 UI 通常不会提供 `fixture_echo`。Root 需先审阅客户端实际提供的 schema，选择已授权的无副作用工具及满足 schema 的纯测试参数，然后用以下参数重新启动；无需改源码：

```powershell
# 以下名称仅为示例，必须换成隔离环境中已确认可用且安全的真实工具名。
node compat/qoder-cn/fixture.mjs '--tool-name=fixture_echo' '--tool-args={"message":"fixture-safe"}'
```

重新启动将分配新端口，官方 UI 的 Base URL 也需更新。若启动前已知安全工具名与 schema，可直接用带参数命令一次启动并完成三个模型测试。PowerShell 需保留传给 Node 的 JSON 双引号（PowerShell 7.3+ 原生参数模式；其他 shell 请按其引用规则处理）。没有确认安全工具时保留默认，不使用写文件、shell、网络或未知工具。

工具证据分层：日志 `tool_call_sent` 只证明夹具发出 call；UI 显示工具请求/执行与对应 call id、客户端下一 POST 的 `toolResultMatched:true` 及最终 `FIXTURE_TOOL_RESULT_RECEIVED` 可证明往返。最后标记只证明收到字符串结果，错误字符串也能到达，成功执行须另看隔离工具的成功证据。call id 一次消费，重复或未知结果拒绝，不能拿旧结果重放制造成功。

## 有界观察与停止

stdout 不写请求/响应全文、Authorization、API Key 或工具结果内容。启动行只显示公开测试 Key。请求行仅保留：本地序号、已知 route/method、认证存在/匹配布尔、固定模型、stream、字节数、角色序列、前 16 个工具及其前 16 个 required/property 名和类型、状态/outcome、匹配结果字符数、SSE 帧数及终止事实。名称限 80 个 ASCII 标识字符，其他值省略。不保存日志文件；Root 可仅收集这些 stdout 行作验收。

1 MiB 请求体、128 messages、64 tools、200 requests/启动、16 pending calls，至多 201 条请求观察（最后一条是限额拒绝）。只监听 `127.0.0.1`，15 分钟自动退出；完成后 Ctrl+C，并实际检查连接已拒绝。Windows 自动化终端的 Ctrl+C 本轮未关闭原生进程，不能只据终端反馈声称已停；必要时在另一终端用 `Stop-Process -Id <本次 started.processId>` 关闭这个已确认的夹具 PID，再验证端口。未知路由/协议、压缩 body、错误凭据、未知模型/结果都拒绝。没有自动重试、请求回放、持久凭据或用户配置写入。

夹具/offline 通过不等于 Qoder 实机通过，也不等于 CC Switch 生产转发通过。原生 BYOK 保存/重启持久化、取消/超时行为、完整错误兼容、vision/thinking、Responses、Anthropic 和 Provider 切换仍未测。

源码边界：准确 checkout 的 CodeGraph 当前 source 显示 `src-tauri/src/proxy/handlers.rs:762` 的 Chat handler 在 780–798 行绑定 `AppType::Codex`；Responses 的 `handle_responses_for_app` 在 849 行提供 app 参数 seam。这支持独立验证客户端协议，不能复用 Codex 凭据来声称 Qoder 专属生产入口已成立。
