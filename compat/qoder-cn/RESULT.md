# 第一包执行回执：2026-10-02

交付为本机隔离协议夹具，唯一写集是本目录。实现：fixture.mjs；可运行检查：check.mjs；实机前置与验收说明：README.md。Node v24.16.0，无依赖下载或常驻服务。未操作 Qoder、读取账号/真实 Key/任务正文、调用真实模型/网关、改全局代理/TUN、Core、正式 CC Switch 源码、registry 或客户端配置；未 Git 提交。

实际命令：

```powershell
node compat/qoder-cn/check.mjs
```

实际标准输出与退出状态：

```json
{"result":"PASS","node":"v24.16.0","scope":"local synthetic offline only","requests":15,"checks":["model catalog","text JSON","incremental text SSE","JSON tool call/result","incremental tool SSE/result","no replay","auth rejection","unsupported route/model","malformed/oversized body","content/key redaction"]}
{"fixtureClosed":true}
{"cliStarted":true,"cliChildExited":true,"cliPortClosed":true}
```

退出码 0。每条 SSE 至少三次网络读取，首末读取间隔至少 100ms；字符串增量与 finish_reason / DONE 均检查。JSON/SSE 工具参数重组后合成一次 role=tool 结果，按 call id 和 assistant 回显匹配，一次消费后重放拒绝。私有正文/Key 检查使用专门合成标记，观察记录中不出现这些标记。此处的 PASS 是该脚本断言通过，不是第三方认证或真实客户端验收。

额外 CLI 观察：曾由 Windows 自动化 PTY 启动 fixture，获得 127.0.0.1:62853。Ctrl+C 未关闭监听，PowerShell CIM/网络查询受权限限制，不以其空输出证明关闭。直接 TCP 连接确认仍监听后，netstat 按精确本轮端口找到 Node PID 39184、启动时间 11:32:04；只关闭该进程，随后直接 TCP 连接返回 ECONNREFUSED。源码启动行现显示自身 processId；离线检查使用自己创建的子进程、关闭并等待退出，最终连接拒绝。没有保留测试监听器。

源码取证：先以准确 projectPath E:/Programming/AI/Agents/Tools/cc-switch 调用 CodeGraph，源证据定位 Chat handler 的 Codex AppType 绑定及 Responses app 参数 seam。CC Switch HEAD/clean 来自 Root 本轮核验；本包没有把它升级成生产运行证据。

真实 Qoder 下一步：Root 用 README 的一次启动命令，在隔离 workspace 的官方自定义模型 UI 选择 Chat Completions，填打印的临时 /v1 Base URL、公开测试 Key 和三 Model IDs。文本/SSE 用固定标记与对应日志、UI展示联合判断。工具默认 fixture_echo 若未提供会拒绝；只有 Root 已确认安全工具 schema 和参数后，才用启动参数指定该工具。没有确认实际工具时保持工具未测；不为测试创建或执行未知工具。

状态边界：fixture/source/offline 已验证；Qoder 官方 UI 接受 HTTP/loopback、真实请求、工具执行成功、BYOK 持久化和 CC Switch 生产转发未验证。Responses、Anthropic、vision/thinking、取消/超时/完整错误兼容均未测。

执行复核：阶段目标与唯一写集匹配；有效进展是实现、离线协议断言与真实子进程生命周期证据；未知项留给 Root 的 UI 验证，不扩展注册/配置/生产功能。完成后停写。

模型证据：父任务请求 gpt-6.1-sol/high。本实现者未获得可独立查询的实际模型标识/推理强度回执，因此不将文字自报作为路由达标证据。
