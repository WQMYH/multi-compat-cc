# Qoder CN 兼容开发交接

更新：2026-10-02（Asia/Shanghai）。源码、离线、安装及实机分别记实。

## 仓库与责任

- Fork：`https://github.com/WQMYH/multi-compat-cc`。
- 源头/upstream：`https://github.com/farion1231/cc-switch`，保留历史、许可与署名，不向源头推送。
- 接续分支：`codex/qoder-cn-compat`；本地起点为 `7c0d0fc615f7e9391de9784958c918368455cbcc`。
- 云端开发会话承担本仓库的兼容实现与离线验证；原兼容准备会话转为信息搜集、核实和整理，不与作者争改源码。
- 本机协调方保留原生客户端实验和独立接受责任。云端无法访问本机桌面、私人配置或临时目录，不能冒称本机已验证。
- agent-core / 多智能体信箱另有写集，未提交到此仓库。本任务不另造客户端、消息平台或重复网关。

## 目标与边界

先完成 CC Switch 与 **Qoder CN 独立桌面版**官方自定义模型入口的兼容闭环，复用既有 provider、协议与网关，再据证据实现最小生产适配。standalone、IDE、CLI、Work/Wake 不互相替代。

当前最小协议为 OpenAI Chat Completions：文本、SSE、一次安全工具往返。Responses / Anthropic 不属于已实现的夹具能力，不硬锁产品版本；版本更新只在相关功能失效时定向核实。此前本机取证对象 `qoder-cn/0.4.3` 不是跨版本保证。

不读取、上传或改写真实密钥、正式配置及默认模型，不改全局代理/TUN，不重放未知请求，不启用 Fast 或扩大付费预算，不自动合并上游或发布版本。

## 已完成与未完成

`compat/qoder-cn/fixture.mjs`、`check.mjs` 逐字节迁自已独立 SOURCE/OFFLINE PASS 的冻结包；只修改文档命令为仓库相对路径，没有生产 CC Switch 代码改动。

历史 Node v24.16.0 的 15 次合成请求通过：模型目录、JSON 文本、分段 SSE、JSON/SSE 工具 call/result、重复结果拒绝、认证拒绝、未知路由/模型、畸形/超限 body 与正文/Key 脱敏。夹具、CLI 子进程及监听端口均有关闭证据。历史回执见 `compat/qoder-cn/RESULT.md`；这是合成检查，不是 Qoder 原生工具、真实上游或生产转发验收。

迁入后从仓库根目录执行：

```sh
node compat/qoder-cn/check.mjs
```

Fixture 仅监听 `127.0.0.1`，公开虚拟 Key，无真实上游/日志文件，15 分钟退出。默认 `fixture_echo` 是合成检查用工具；真实客户端未提供时明确拒绝。只有核实客户端 schema、安全工具及参数后才可调整启动参数，不为测试执行未知写入、shell 或网络工具。

## 可复用的源码事实

- 上述 checkout 的 `src-tauri/src/proxy/handlers.rs` 中 Chat Completions handler 绑定 Codex `AppType`；不能借 Codex provider/凭据冒称 Qoder 专属接入。Responses 的 app 参数路径只是可复用 seam 的线索，改动前仍须核对当前源。
- 本机安装件及官方 UI 输入校验允许 HTTP/HTTPS Base URL，拒绝内嵌凭据、query、fragment；这是静态校验，不是 loopback 实测。
- 该安装件接受 `openai`、`openai-responses`、`anthropic`；`validateAndAdd` 成功会继续创建 BYOK 配置、保存 metadata、刷新 projection，不能称为“仅验证，不保存”。
- 稳定公开的 BYOK 持久化自动接管契约尚未证明，不猜写原生私有文件、IPC/RPC 或数据库。
- 用户已允许通过官方 UI 添加/删除仅 loopback、虚拟 Key 的临时模型配置和新测试会话，不动既有模型、密钥及默认选择。实机仍等待本机切至设置/自定义模型页，不能绕过私人聊天采集拒绝。

## 云端下一步

1. 确认实际执行环境、fork、分支/提交和工作树；读取本文件及 fixture README，不将上游代码称为本任务新实现。
2. 定向验证迁入路径及云端 Node 行为，记录实际测试、子进程/端口退出；不重复无环境变化的旧接受。
3. 核对既有 provider/模型入口及网关 seam，提出最小实现/测试差异。机制不清先查当前官方资料或源码；同写集仅一个作者。
4. 独立可做的源码和离线反例在 `codex/` 分支有界推进；依赖真实 BYOK 契约/客户端结果的部分明确所需证据与本机接续动作，不降低验收。
5. 交付准确提交、差异、实际测试输出和未决项；独立接受后才安排本机验证，不自行扩产品、合并或发布。

用户已允许 CC Switch 初始化 CodeGraph，索引放 Git 仓库外；本机 `.codegraph` junction 不上传、不证明云端有索引。新 Sol 执行采用 `gpt-6.1-sol` 并保留适用推理强度；不因等待升档或重做已完成工作。
