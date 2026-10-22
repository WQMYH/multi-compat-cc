# Qoder CN 兼容开发交接

更新：2026-10-18（Asia/Shanghai）。源码、离线、安装及实机分别记实。

**当前目标以本文末尾「三向接入与官方账号授权管理目标 v1.0」为准。** 用户已明确官方账号管理与第三方模型接入都是目标；此前只围绕桌面BYOK网关的设想不是完整目标，也不是已实施的生产方案。

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

## 2026-10-17 本地上游更新

用户明确要求先更新 CC 本地源码，再继续 Qoder 接入与账号授权控制开发。

- 上游 `main` 已取回到 `797e0276ffc20e54cbdf55e3de84505d77231c91`，包版本 **4.0.14**。相对旧基线新增500个提交，涉及1008个文件；原3.20.4的协议、provider与鉴权 seam 结论必须重新核实，不能当作4.x现状。
- 原 `codex/qoder-cn-compat` 分支及其 `be75253` 提交保留不动。当前本地开发分支为 `codex/qoder-cn-compat-upstream-20261017`，从最新 `upstream/main` 创建，并用 `cherry-pick --no-commit` 带回原五个已接受的夹具/交接文件。未创建新提交或推送；后续若要提交，不要误将代码图 junction 或本机材料加入发布。
- 原五个文件的Git index blob与旧提交一致。Windows检出使部分工作文件变为CRLF，首轮逐字节磁盘比对因此未通过；定向复核确认只有换行差异，没有夹具代码变化。本文随后追加本节，属于本次明确的文档增量。
- `.codegraph` 仍指向仓库外原索引位置，按本次源码更新同步；旧图不能替代新版源码。依赖安装、编译、客户端安装、Qoder实际协议及账号授权控制均尚未因此通过，不重复运行未变的旧夹具来冒充这些结果。
- 云端兼容会话当前从本机不可读；fork上原兼容分支仍为 `be75253`，未见更新提交。本地更新不宣称接管或完成云端未取回成果，不向上游推送。

下一步只围绕4.x的真实provider/协议入口和Qoder官方账号契约核实必要改动；不读写真实密钥、产品私有凭据、默认模型或正式账号状态。

本次仓库外 CodeGraph 已同步732个变化文件、19414个节点。定向读取4.0.14当前源码确认：`src-tauri/src/app_config.rs` 的 `AppType` 枚举有10种产品，尚无Qoder；不能把其他产品的登录管理当作Qoder账号支持。新版provider代码已存在 `mode` 契约和 `live` 文件变更规划，Codex专用路径还会校验登录身份与钥匙串状态。这些只说明需要沿用的新架构边界，不证明其可直接用于Qoder，也不授权读取真实登录材料。后续兼容应在新的产品分支中复用相应规划/回读机制，避免将旧3.x直写假设硬搬进4.x；本次未修改这些生产源码。

## 2026-10-17 当前本地实施包

用户已直接要求继续4.x的Qoder CN接入与账号授权控制、必要修改/回归及独立复核，明确禁止改动真实账号配置或凭据。当前本地源码由Root单写；独立审阅者不修改候选。此前云端职责与旧夹具的接受仅为历史，不证明新4.x实现已经交付。

先核实的决定性差异：4.x的默认`/models`实际读取Codex配置/目录，Chat Completions仍固定Codex身份；给该入口加Qoder路径别名不构成产品隔离。Qoder接入必须有独立provider/目录/授权边界，保留既有Codex行为与账号材料。

2026-10-17重新核对的官方来源：

- https://docs.qoder.cn/qoder/custom-models ：桌面支持官方UI中的自定义Base URL、API Key、Model ID及兼容协议；验证成功会保存，不是只读预检。该公开页没有提供可供本实现接管的桌面登录凭据或BYOK文件写入契约。
- https://docs.qoder.cn/cli/authentication ：`/login`、`/logout`、`QODERCN_PERSONAL_ACCESS_TOKEN`及`QODERCN_CONFIG_DIR`是CLI契约，不能冒称桌面账号自动切换能力；Qoder账号认证与第三方模型API Key分开处理。

本轮验收以可到达的独立入口、真实选中provider隔离、未授权调用拒绝且不触及其他产品、受影响回归与独立接受为准。只用合成配置/隔离数据库和loopback上游；不启动真实产品、不查询或修改用户账号/凭据/默认模型、不提交或推送。旧协议夹具保持复用，不能重跑未变夹具替代新增生产路径的验证。原生桌面账号接管在没有契约时保持明确不支持，不猜私有文件、IPC或借用其他产品OAuth。

## 三向接入与官方账号授权管理目标 v1.0

制定：2026-10-17（Asia/Shanghai）；依据用户本轮连续澄清及「结合夹具测试结果订立目标」要求。本节取代把账号控制理解为仅管理第三方API Key、或仅实现Qoder作为下游客户端的范围。目标已建立不等于任何新能力已实现。

### 用户结果

在CC Switch 4.x中形成一个有明确账号身份、调用方向和能力边界的Qoder CN接入面，复用现有认证、provider、网关、日志和切换机制，不另建账号中台、消息队列或独立客户端。

| 方向 | 必需用户结果 | 不能混称的能力 |
| --- | --- | --- |
| Qoder官方账号 → Codex及其他兼容客户端 | 通过官方SDK/公开认证机制管理选定Qoder授权来源，并验证能否作为目标客户端的供应商使用 | SDK调用完整Agent不自动等于原始模型API；Agent委派不能冒称Codex模型已切换 |
| Codex/ChatGPT等已授权供应商 → Qoder | 由CC提供独立Qoder网关，按真实获授范围将选中供应商接入Qoder官方自定义模型入口 | 已有Codex登录不自动证明具有第三方应用推理授权；不复制宿主登录文件或把身份登录冒称推理许可 |
| 第三方模型 → Qoder | 管理独立provider和模型配置，通过官方BYOK入口使用 | 第三方API Key不是Qoder官方账号登录或账号切换 |

这里的「接入」指可用能力和推理调用，不指共享聊天历史、复制桌面会话或接管当前根会话的宿主模型。

### 现有证据和不能跨越的差距

- 旧夹具已接受的15次合成请求覆盖模型目录、文本JSON、分段SSE、一次工具往返、鉴权拒绝、重放拒绝、输入限额、脱敏及子进程/端口清理。保留`RESULT.md`原回执，不因订立新目标重跑。它们没有证明CC4.x生产路由、Qoder实际消费或真实账号授权。
- 4.x当前默认Chat/Responses及models路径具有Codex身份/目录耦合。独立Qoder入口不得仅添加别名、继承Codex当前provider或回落到其登录态。
- CN官方SDK认证公开了显式PAT、Service Account及`qodercliAuth()`；PAT失效需重新提供，不能承诺SDK自动刷新PAT。Service Account代表组织工作负载，不冒充个人账号。CN/Global包和端点分开核实，不因CN示例出现Global域名就自动跨区。
- SDK公开`startup()`无用户消息/模型请求的握手及Query上的`accountInfo()`账号元数据。后者可为空/回退旧CLI初始化快照，不返回凭据；不能承诺WarmQuery直接查询账号。随后核对的CN发布包1.0.53还公开`startDaemon()`及daemon级`accountInfo/getModels/getUsageInfo`和BYOK操作，要求协商`daemon_queries_v1`，失败直接拒绝；详见下方新回执。发布包明确login/logout/status由CLI提供，未发现SDK原生浏览器login/logout/listAccounts/switchAccount；不能靠函数命名或私有文件猜测实现。
- 官方OpenAI文档已提供获授ChatGPT计划调用的公开Responses路径；实现仍须核验CC当前授权类型、实际获授权限和协议要求，不将旧OAuth凭据或目录可见性直接当作可用资格。

### 实施顺序与门槛

1. **认证和协议可行性先行。** 核对CN官方SDK的发布包、认证构造器、账号快照/失效回调、会话关闭，以及CLI官方登录入口；同时核对CC4.x当前认证/网关消费者。先回答Qoder能否满足目标客户端的消息、流、工具和终态语义，再写大规模AppType注册。只读/初始化不等于模型调用成功。
2. **Qoder官方账号授权管理。** 设计并实现显式授权档案、身份/状态显示、选择、失效处理和解除绑定；优先官方登录/凭据委托，不读取或复制私有登录缓存。SDK没有登录或登出接口时沿用官方原生入口并准确标示限制，不自造OAuth。CC档案切换只影响后续新会话/请求，不偷换在途账号。CC解除绑定、供应商撤权、CLI/桌面全局登出分别呈现；不能用其中一个冒充其他效果。
3. **逐向适配。** 先按真实契约打通Qoder账号对外能力及独立Qoder下游网关，再接现有Codex/ChatGPT获授供应商和第三方API Key。Qoder与其他产品分别保存provider指针、模型目录、请求归属及访问凭据，不污染已有登录/默认项。复用已有协议转换；只补被证明缺失的接缝。
4. **回归、独立接受及分层交付。** 对新代码先补反例/隔离回归，再冻结候选交未实现者审阅。源码、安装、真实客户端、真实账号/业务验收分别记录。当前阶段交付为源码与隔离验证；实机账号授权和真实推理必须另有明确对象与有效授权，不能自动操作现有真实账号。

### 必需验收

- 官方账号与第三方Key不混用；空账号快照不报已登录；过期/撤权明确提示，不切换到另一个账号或区域，不重发未知外部动作。
- 多账号、并发请求及会话切换保持身份绑定；一次选择不能在await后变成另一账号；解除绑定不删除其他产品材料，不影响已派发动作的事实记录。
- 对外作为模型供应商时，至少验证文本、流式结束/失败、一次调用方工具call/result及多轮关联。若SDK内部执行了工具或只提供Agent委派，必须如实区分，禁止吞掉工具语义后声称模型兼容。
- 下游Qoder网关走实际生产router和独立provider/模型目录；未授权请求在外发前拒绝，无可用Qoder provider时不回落Codex；入站网关凭据不传成上游认证。Chat/Responses/Anthropic按实际实现分别标支持/不支持，不能用一套协议通过外推全部。
- 用合成账号/隔离存储证明真实账号配置及凭据零改动、日志不含token或正文、回归不损坏原Codex/其他产品路径。旧未变夹具只复用；新增生产路径必须新增对应证据。
- 不支持的必需方向保持未完成并给出决定性依据、可行替代及下一责任；不能用Agent委派替代原定模型兼容后将目标标为全部完成。

### 当前授权与责任

本轮Root唯一修改本地CC源码及交接记录；既有独立审阅者只读。用户明确禁止修改真实账号配置或凭据，因此本阶段不运行真实登录、退出、换号、令牌刷新或计费模型调用，不导出真实token，不向上游提交/推送，不重放任何旧未知请求。需要实机授权时只停相应支线，继续无关离线工作；不新增自造短期授权票据、全局代理/TUN或自动关机。

当前实际准备：冻结lockfile依赖安装完成，4.0.14前端基线typecheck退出0，package/lockfile未修改；Rust固定工具链准备遇到组件缓存rename失败，基线编译未通过/未判定源码错误。已停止本任务产生的单个持续恢复工具链清单进程，保留失败和部分缓存，不盲重装或改仓库工具链约束。后续先核实已有可用编译器，再决定相称的本地验证路径。

本轮反思与下一动作：用户澄清后已停止把纯桌面BYOK网关视为完整方案；仅文档改变，尚未修改生产实现。下一高价值动作是官方SDK发布包与CC认证契约的无凭据兼容探针，明确原始模型接口与Agent委派的分界，再确定最小写集。新目标保持进行中；订立目标不算实现完成。

### 决定性官方来源

- [CN SDK认证](https://docs.qoder.cn/cli/sdk/authentication)及其同源`.md`正文。
- [CN TypeScript Reference](https://docs.qoder.cn/cli/sdk/references-typescript)：startup、Query.accountInfo和auth类型。
- [Qoder CN桌面自定义模型](https://docs.qoder.cn/qoder/custom-models)：官方UI配置、验证与保存。
- [OpenAI Sign in with ChatGPT：模型与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)：获授计划调用、公开Responses路径和完成事件；具体限制还需随适配核验。

### 2026-10-17 SDK发布包核对与隔离契约探针

目标和三向必需验收不变。本轮新增无凭据SDK探针，不把它称为生产账号管理、网关或原生推理已完成。

- 官方CN npm包`@qodercn-ai/qodercn-agent-sdk@1.0.53`归档173968字节，SHA512与npm元数据一致；包内声明配套CLI1.1.71。未执行postinstall/下载或启动原生CLI。SDK自身许可由Qoder产品条款约束，不能因CC仓库MIT就将SDK改称MIT。
- 独立静态复核已回收：`Daemon`账号查询/BYOK接口可用，daemon-bound Query必须省略auth并继承启动身份。账号档案切换必须遵循独立daemon生命周期；不能篡改单条Query的auth冒充共享daemon换号。`hostActionLaunch`是内部启动上下文，不是登录API；`canUseTool`只有allow/deny，不是工具执行结果回传。
- 新增`compat/qoder-cn/sdk-contract/`，使用真实官方SDK，只模拟stdio原生端点。根`package.json`增加开发依赖及`test:qoder-cn-sdk`；工作区禁用该包postinstall，lockfile固定此次探针依赖。没有新增生产AppType、账号面板、代理路由、队列或服务。
- 首轮六项中四过两失败，失败均为测试错误地要求Windows强制结束的子进程仍生成JS退出文件；保留首次诊断。修正为检查本任务启动的进程确已退出，并保留正常退出码检查；随后六项通过。再补Query身份覆盖拒绝及缺凭据不启动两项，最终结果与独立接受见后续回执。旧15请求BYOK夹具未重跑。
- Rust编译另核实可用1.96.1编译器后，`cargo check --lib --locked --offline`因本机索引缺`arboard`未进入源码编译；不是源码失败或通过，也不为此重试损坏的固定工具链缓存。前端基线typecheck已有退出0证据，新增依赖的受影响结果另记。

复核/下一动作：最近有效进展是消除SDK账号查询与身份绑定的不确定性，并形成可执行反例，而非重复目录或ACK。原始模型供应商的任意历史及调用方工具暂停/续跑仍缺契约，不能用Agent委派替代。先收新探针最终结果并冻结给原独立reviewer，再据此确定最小生产账号接缝；账号真实登录/推理、产品安装、提交/推送仍未获本轮授权。

SDK account contract **source1已冻结**：Node24.16.0下8项全部通过，0失败/0跳过；受影响`pnpm typecheck`退出0，`git diff --check`无错误。首轮失败证据没有删除；Windows异常终止与正常关闭继续分别检查，不降低SDK协议门槛。原独立reviewer `/root/ccswitch_4x_delta_review`已接只读delta，尚未接受时不称独立通过，也不据此安装产品。

冻结写集：`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`及`sdk-contract`下两个mjs和README。账户探针SHA256为`76e47996461c1e7488c5a812f81688ddbc689fbe82dd154e2a86e6ac7801f5a8`，原生端合成夹具为`1b5b14e7b06814e837eb4d4b1e91c0b4a1d5ecae572bda26000ffb47834d3d1a`；根文档是Root可追加的状态记录，不作为这六个冻结文件之一。当前仅新增开发/验证代码，生产三向接入仍未交付。

### 2026-10-17 探针接受与授权档案生产接缝

原独立reviewer已对上述SDK探针给出**SOURCE/STATIC ACCEPT，无阻断项**；六个原冻结文件及安装SDK入口相符，确认真实SDK、合成原生端、错误/进程/日志检查的证据界限。8项通过及typecheck沿用Root回执，reviewer未重跑，不冒称原生实机。后续新增生产打包脚本使`package.json`变化，此接受不自动覆盖新写集。

当前Root新增`account-profiles-source1`工作候选（尚未整体接受）：

- 复用既有授权中心，增加Qoder CN授权档案界面和五个对应Tauri命令；复用现有CC数据库settings单一命名空间，不另建账号数据库/服务。保存用户明确的PAT环境变量**名称**或指定CLI配置目录，不保存令牌值、复制登录缓存或修改原生账号配置。
- 通过官方SDK的短生命周期daemon核验正向userId，保存有限显示元数据；选择档案不等于切换桌面登录态。确定的凭据/身份失效清除选择但保留上次身份历史；连接超时只记录本次核验失败，不伪称账号过期或擅自换号。解除绑定只删除该CC档案，不撤销供应商凭据或自动选择另一账号。
- 选定身份再次核验时必须匹配，编辑/删除/并发核验采用revision检查，await后的旧结果不能恢复被删除档案或覆盖另一身份；档案编辑会清除此前核验与选择。新增五项Rust隔离数据库回归，但目前尚未执行，不以源码存在冒称通过。
- `src-runtime/qoder-auth-host.mjs`使用官方SDK；Vite既有构建将其打包为Tauri资源，不进入浏览器bundle，不执行SDK postinstall，不另安装Node或原生CLI。Node沿现有安全PATH定位机制或用户指定绝对路径；Qoder CN CLI需用户明确指定，不硬编码版本目录。沿用现有子进程树隔离与deadline清理，不放宽原工具定位行为。
- 生产helper隔离检查首次1过2失败，定位到SDK的`env`是替换而非合并；已在helper显式继承环境，原生CLI身份分支再明确去除环境PAT干扰。修改后3项全部通过，包括真实构建产物调用、PAT/原生CLI身份两种合成端路径、空/变更身份拒绝、缺凭据不启动，以及输出不泄露dummy token/额外账号字段。
- 界面2项通过，证明加载不启动登录/核验、未核实不能选择、显式核验后选择及只保存引用；TypeScript检查退出0，生产renderer和SDK资源构建退出0。首次类型检查发现当前Vitest不支持新matcher，已改为既有计数/参数断言，后续退出0；这不是放宽业务断言。

Rust依赖下载已通过当前1.96.1工具链执行`cargo fetch --locked --target x86_64-pc-windows-msvc`，handle8461最终退出0。首次编译因构建进程没有MSVC环境而报`link.exe not found`，后在本机找到已有Visual Studio 2026及Windows SDK，仅在构建子进程中加载官方VsDevCmd，handle34997单job编译在途；无新增工具链安装、全局环境修改或重复下载。索引/依赖准备和编译是不同状态，目标目录在独立临时盘，不修改仓库固定工具链文件。

本轮匹配复核：上一目标轮有真实SDK契约及独立接受进展，本轮已增加实际CC授权中心/命令/SDK构建接缝；不是重复目标或纯状态汇报。下一步完成Rust编译/五项后端回归，针对新冻结候选安排独立delta复核；随后继续独立Qoder下游网关与三向协议/供应商接入。官方CLI登录/退出启动、完整模型供应商语义、Qoder生产router/provider隔离、获授OpenAI计划调用和真实账号实机仍未完成，保持原目标进行中；不能拿账号面板或合成端通过代替这些必需项。

`account-profiles-source1`已交原独立reviewer只读审阅，核心冻结SHA256：SDK helper `c2989e7cc58bd4e9025d496b67ec0be670bfc1d41716a391ca3a9a4b93790ba6`、Rust命令与档案逻辑 `d04658d3f0becddd828c137e4ac356990b331789c6ac752bcc56ec7633e9b001`、界面 `032adc80650a65c03028dfd1b9307fffb3ffaeecb0b5ae5d7bb1c6f9a40e5249`。SDK资源附带原始Qoder SDK LICENSE并比对相同；实际构建及10项受影响前端检查通过（新界面2、原三组授权8），不把这些结果加总当成后端或三向接入通过。仍不安装、发布或使用真实账号。

### 2026-10-17 account-profiles-source1复核与source2清理修正

原独立reviewer收口：source1仅有一项P2阻断——helper在全局临时目录另建diagnostics/config，Rust TempDir只作cwd；父级deadline终止helper后JS finally不能执行，且stop抛错会跳过rm。其余账号引用、revision与身份回写、UI/注册/构建接线未发现新增SOURCE/STATIC阻断；此结论不覆盖尚未执行的Rust回归。

Root已开始source2，旧source1冻结与finding保留：父级拥有唯一scratch，同时将helper进程TEMP/TMP/TMPDIR指向它，覆盖SDK自身os.tmpdir认证载荷；helper内部只在该scratch建诊断子目录，并用嵌套finally保证stop异常不跳过清理。追加真实生产helper/官方SDK/合成原生端的deadline回归，要求原生accountInfo已到达、超时后父级临时材料已回收、显式原生配置仍保留；不是以正常退出代证异常退出。源码和新构建尚在验证，未将source2标为冻结、通过或安装。

`account-profiles-source2`随后按四文件明确冻结并取得原独立reviewer的**affected SOURCE/STATIC ACCEPT**，无新增阻断，source1 P2在源码层关闭；旧其余接受保留，不重复全包/SDK/15请求探针。冻结SHA256：helper `a28081f4ac201b62d2107bba97b9e7baf32b64060b78859d61f7c57231121749`；Rust `6313ca3519b4d9e2a112b26cdcf3cb892d0c24a578db49d2aa55ec298e8220dc`；host fixture `d4e16cb57ae934b13ebfd5b098da5c712562eed3508cbc5e93a2bd2505113c1e`；host test `92efd9c278a1fd1e118ef4444b6dfdcb209a2836be919fcdcce583d2963435df`。构建后的生产helper三项隔离检查退出0；reviewer仅读源码与既有证据，Rust六项（含新增deadline）仍待同一在途编译结果，不称OFFLINE全过或安装/原生账号可用。

连续执行复核：最近新增价值是P2根因及修正、独立delta接受、生产helper回归；现有单job Rust编译持续推进，不另起并发构建。SDK文档的`models.read chat.completions`scope示例不能单独证明CN原始模型端点/请求与客户端工具循环，保持原始模型兼容方向未完成；OpenAI官方公开流程另明确需要实际推理scope及公开Responses完成事件，不能直接把旧Codex登录态当作具备该授权。

#### 现有CC认证消费者的具体接缝

当前4.x `src-tauri/src/proxy/providers/codex_oauth_auth.rs`的`start_device_flow`使用固定Codex client与deviceauth路径，`CodexAccountData`保存稳定账号/workspace、refresh/id token及更新时间，但没有新公开计划调用所需的issued dynamic client、host ID或granted scope登记。`exchange_code_for_tokens`仍走原Codex的token URL/redirect组合。因此这条既有Codex认证不能直接作为Qoder下游的公开ChatGPT计划推理授权。

OpenAI官方《Registration and sign-in》《Models and inference》本轮公开文档已读：独立开源客户端需动态登记、验证ID token的签名/issuer/audience/nonce以及实际`chatgpt.tokens.use.direct`scope；推理走公开`/v1/responses`、`store:false`、`stream:true`并等待`response.completed`。后续可复用CC既有认证基础与provider界面，但不能更换几条URL后假称原有令牌已获授，也不改现有Codex认证使旧用户被迫迁移。本轮只读取源码/公开契约，未读取实际账号、发起登录/refresh或模型请求。

下一生产包应先形成独立Qoder provider/模型与网关上下文，再接明确API Key供应商及具备上述授权的计划供应商；不把默认Codex路由换别名，也不以尚未交付的账号选择面板代替请求消费者。

### 2026-10-17 授权档案后端通过、独立网关开始接线

上一目标轮归类为**progress**：source2修复、独立delta接受和生产helper验证增加了可核对证据；不是状态重述。原在途Rust handle34997最终退出0，测试profile首次完整构建27分28秒，`commands::qoder_auth::tests`六项全过，含真实生产helper/官方SDK/合成原生CLI的强制deadline清理（先到accountInfo、父级scratch消失、显式config保留）。因此该候选现有SOURCE/STATIC接受及上述OFFLINE证据均成立；不称真实账号、安装或三向整体完成。原失败和source1 finding保留，旧15请求不重跑。

本轮新增独立网关工作候选，尚未冻结/独立接受：

- `qoder_gateway.rs`复用CC providers表，以`qoder`命名空间保存显式API Key供应商、选择和模型目录；只读摘要不回传上游Key，未选供应商不取Codex默认项，不写任何原生产品文件。网关Key仅用户显式准备时建立，无自造短期到期票据，未鉴权请求不能武装连接。
- 生产CC router合并`/qoder/v1/models`、`/qoder/v1/chat/completions`及`/qoder/v1/responses`。入口直接取得Qoder-owned provider快照，首个await之前固定身份及凭据，不借AppType::Codex或其native目录。复用CC既有raw transport、代理选择、响应流和日志；绕开可能在POST效果未知后raw→hyper回退的分支，不添加全局调度、队列或第二发送状态机。
- 此最小网关按Qoder官方BYOK选择匹配API同协议直通，保留原消息、调用方工具与SSE；格式或模型不符在外发前拒绝。不重造协议转换；当前API-key路径不自动接受旧Codex OAuth或伪称获授ChatGPT计划调用，后者仍须独立补齐公开授权消费者。
- 授权中心新增明确区分官方账号与BYOK的配置区域，复用既有代理服务的显式启动命令；加载仅列配置，不自动启动/推理。新入口及三向目标不自动增加CC的原生文件/MCP/Skills支持面。
- 新Tauri/前端接线后typecheck已退出0。新增生产router隔离回归覆盖前置拒绝、真实HTTP凭据归属、工具往返/分段SSE、Responses终态、await期间切换不改原请求及上传后断连不重试，尚待实际执行，不以测试代码存在称通过。

资源协调：既有单job构建已正常结束；2026-10-17T11:05:06+08:00现场物理余5942460416、提交余7174156288字节，无本任务编译进程残留。已在本根回复中台相关请求，不推送或恢复别的业务。后继仅必要单job增量验证，不并行整包构建/模型实验、不改全局资源设置或强杀其他任务。新网关尚未安装/用于真实账户；Root保持唯一源码及计划写者。

#### qoder-gateway-source1冻结及待收

新界面三项全部通过，随后typecheck退出0。首次Rust增量handle36576因测试使用未启用`util`的`tower::ServiceExt`而编译失败，未执行新测试；保留原错误。改为既有`Service`加`poll_ready`的owned dispatcher，没有新增依赖、启用Cargo功能或降低断言。同时修正两个测试夹具问题：按随机UUID排序取last不代表新供应商，工具回合须按末条role区分首发与tool result。现重新提交的是修正后的单job增量handle74481，执行`--lib qoder_gateway`及`--lib proxy::qoder`，旧6项账号/8项SDK/15请求不重跑；handle当前确在编译，尚无新后端通过回执。

`qoder-gateway-source1`已冻结并交原独立reviewer只读delta：主配置/DAO模块`f56a4943edd19353e4dd0f8e5faa449a66b1ad91e5fcf9715dee884b01334e8b`；生产路由及五项测试`19d49b7c224e04c547a487981b5a3fb69207ce36f36f96978056dafb34343490`；Tauri命令`72ac101cab1cf43fe279eed79eb74868f17b278b9116a0d15af9398fddbf58e1`；UI `8d6bdbe256191ceb3364610f0ed28ca8a69df3a2dbc78bf7138fab79a8da47bd`；typed API `0a07bccb0a006763671520c4db9d18080cdc2deca04b42f57bb610ddbf12342b`；UI测试`39cd461341ee79a0723cde71e9569b924426ce6e138b398451d584d3b09f422d`。其余注册/接线冻结为proxy server `3adce89a1cca8869c2067b9823d23a812b4b1e2436488ed1205751e47b546c84`、raw transport可见性 `7b339a0615be10eafbd9c389e6c9ea3af8900d0775b7ffe0f29207c1513353e9`、lib `f2ccc3c5537a9b9040c2df8f55caca806b38ec242dc47f2124f71606185d8c20`、commands mod `4b7030109850fbd7fdeeb73a9440f9a109d7d97630f524303908ae7e564d524e`、proxy mod `780ac78f912b11785bc58c9b3881f8a976fb25555595d9f54d3f5badf6aa9dcf`、AuthCenter `baa01d5856dd5797f4f6a9b1bdebdc8322295249abaea5d090d9fb3c229ce347`、en `ba5afdef0a5132f3cc308113a93424505ed2d2ef8d10955afefb8e25aa29a268`、zh `6fc180f93d1d3821962afbcf864e1568625f77ca34bc8471eaeef0781bf5fd40`。根交接文档可追加，不在该源码冻结内。

阶段匹配复核：本轮已增加真实生产路由、数据库隔离、显式配置入口及三项UI证据，非重复计划或ACK。等待项只有当前增量回归与该候选独立审阅；不为等待另建作者/重复构建。此包仅目标③API-key子路径，公开获授ChatGPT计划消费者及Qoder账号对外模型供应商仍未实现，原完整目标保持active，不因局部交付缩标或安装。

#### qoder-gateway-source1实际回归与独立结论

原增量handle74481已退出0：三项DAO和五项实际生产router/loopback HTTP回归全部通过，0失败/0跳过；首次增量构建4分52秒，旧账号/SDK/15请求夹具未重跑。三项UI和typecheck沿用本候选已完成结果，不重复计作新增验收。

原独立reviewer收口SOURCE/STATIC：source1暂不接受，唯一确定性阻断为P2。Qoder入口将合法全局`socks5/socks5h`配置交给仅实现HTTP CONNECT的raw transport，故错误握手可达；原八项后端测试不覆盖SOCKS，不能消除该finding。独立命名空间、首await前身份快照、认证头重建、前置拒绝、单次派发、工具/SSE透传及显式UI启动未发现新增确定性问题；不将此局部结论外推真实产品或三向完成。

Root据此开始source2，仅在外发前一次性选择正确的已有传输：直连/HTTP CONNECT沿用raw，SOCKS使用现有reqwest及共享基础配置，并显式禁止重定向和自动重试；不新增依赖、不失败后补发、不改变其他产品客户端行为。新增隔离SOCKS代理的真实握手、POST、流式透传和断连/重定向不重发反例；先完成受影响验证再交原reviewer复核。当前未安装、未用真实账号、未改变全局代理。

连续执行复核：原完整三向结果不缩减，当前仍限定源码/合成端验证；本轮有效新增为source1八项实际后端回执、独立SOCKS根因及新增真实握手回归。新回归首次构建handle43807因抽取函数返回类型未转换而退出1，测试未执行，不能称已证明旧行为失败；已仅对齐错误类型，handle64253继续单job运行同一窄回归。完成原行为负对照后才改传输分支，不以换代理或重复模型请求取绿；旧账号/SDK/15请求证据继续复用。其余两向未实现保留，不因新网关局部成功变更接受条件。

后续公开计划供应商的契约差异本轮已对照[OpenAI官方Preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)：HTTP必须`store:false/stream:true`且携带完整input，不接受`previous_response_id`及多项普通Responses字段；原API Key透传回归中的previous_response_id仅证明该供应商路径，不能移作ChatGPT计划供应商验收。计划授权仍需issued client/host、ID token签名/身份/nonce及实际推理scope，不能借旧Codex缓存或将API Key类别改名。Qoder官方[登录认证](https://docs.qoder.cn/cli/authentication)提供交互式`/login /status /logout`及显式PAT/配置目录，SDK档案选择与桌面换号仍分别记录；当前未执行上述真实账号动作。

#### qoder-gateway-source2冻结与窄回归

负对照handle64253退出1（编译3分59秒、实际运行一项、0.02秒）：隔离代理收到首字节67（HTTP CONNECT的C）而非SOCKS版本5，继而原生产传输报CONNECT read failed。该结果证明新增握手断言能捕获source1缺陷，不将先前类型编译错误冒充负对照。两次失败均保留；未访问真实网络供应商。

source2只改`proxy/http_client.rs`的共享基础配置派生客户端（12行新增，不改旧客户端行为）与`proxy/qoder.rs`的单次传输选择/受影响测试。SOCKS scheme不区分大小写，SOCKS5h域名留给代理解析；显式关闭重定向与底层协议重试，失败不再选另一传输。隔离测试覆盖两种SOCKS握手、完整JSON/认证头/分段SSE、307不重发和完整POST后断连一次性；直连生产router反例继续保留。按本次代码冻结交原reviewer，仅收受影响delta。

handle50526单job执行七项`proxy::qoder`回归；仅成功后顺序构建实际renderer，避免并行重负载。当前在途，源码冻结不代表回归、独立接受或安装。source1既有DAO三项、UI三项及账号/SDK/旧协议夹具未受本次变化影响，不重复运行。

source2冻结SHA256：`proxy/http_client.rs`为`ebf31ca3fe2a2dbdb7689550c16e07a5acd2ded457247d473e881104ff5cb08b`，`proxy/qoder.rs`为`079c7eec4625a4ecc01def72fa9739551f2688317672bf4c44daee16347e6b1b`。原独立reviewer核对一致后给出**affected SOURCE/STATIC ACCEPT**：source1 SOCKS P2在源码层关闭，无新增finding；真实握手、完整POST/认证/SSE和断连计数的观察面成立，普通CC客户端未改变。没有重跑既有测试或将静态结论称作离线通过；Rust与renderer仍等同一handle实际回执。

本次匹配复核：已消费而非重复派发既有独立交付，修复确实对应合法代理可达性与原不重放条件。当前只有必要单job构建等待，未改变模型/权限/账号/产品版本；无证据显示外部绝对阻断。整体目标继续active，剩余官方计划授权消费者、Qoder原始模型契约及真实产品阶段单独保留，不自设新队列、账号平台或以重复探测填充等待。

handle50526的Rust阶段已结束：增量编译4分02秒，七项affected回归全部通过，0失败/0跳过，0.08秒。新增SOCKS5/5h真实握手、流式透传、307单次接收、完整POST断连不重试均通过；原实际生产router五项受影响证据重新成立。source1三项DAO及三项UI保持原接受，不重复累计。随后同一handle开始实际renderer生产构建，显示既有Browserslist/baseline数据与duration类歧义警告；未据此更新无关依赖或扩大写集，构建最终退出另记。

handle50526随后最终退出0：renderer生产构建完成（3456模块、36.91秒），沿用的bundle大小及混合导入警告保留，不为非阻断警告改无关代码。Root接受本次`qoder-gateway-source2`的受影响SOURCE/STATIC与OFFLINE范围；source1 SOCKS阻断已由真实负对照、最小修正、七项回归和原独立审阅共同收口。源码、构建、安装及真实账号/产品效果仍分开，本次没有安装、发布或真实模型调用。

下一责任仍为Root：继续原目标中尚未交付的公开获授ChatGPT计划供应商，以及Qoder账号对外能力的原始模型/Agent分界。前者须按其实际公开请求限制和授权契约接入既有CC认证/provider基础；后者须有决定性公共契约，不能用SDK内部完成的Agent回合替代调用方模型与工具生命周期。先消除高风险假设，再增加最小消费者与隔离反例；现有网关和账号档案未变证据不重跑，不扩大到真实账号。整体目标保持进行中，不标全部完成/暂停/阻断。

### 2026-10-17 公开ChatGPT计划授权协议接缝

上一目标轮归类为progress：SOCKS生产修正、旧行为负对照、七项回归及renderer构建，均已取得原独立接受；没有在途构建待轮询。本轮Root继续唯一写者，先把公开授权协议作为既有providers层的生产模块验证，再接授权中心与Qoder消费者，不缩小三向目标。

当前CC的Codex和xAI登录管理均为各自固定client的Device Code路径；前者的原生配置/续期/默认账号消费者不能接纳新issued client的计划令牌。不能直接在旧账号行添scope便当作获得新授权，也不能借旧私有缓存。选用同一CC授权中心/provider基础里的明确计划授权接缝，原账号不迁移，先验证动态登记、PKCE/state/nonce、signed ID-token/JWKS、真实scope、轮换与错误语义；不新造消息服务或账号平台。

协议将使用已有reqwest、sha2、uuid、base64及锁文件里已存在的ring，仅将必要签名验证库声明为直接依赖，不下载另一套运行时。回归只用合成RSA身份、测试内loopback OAuth端点；不读取真实令牌、原生账号文件或进行任何官方登录/刷新/推理。源码协议通过仍不等于完成账号UI、持久化或网关消费者，后者保留为必需后继，不以无消费者的局部检查称完整实现。

#### chatgpt-plan-protocol-source1实际结果及两项修正

原单job handle81891最终退出0，增量构建6分09秒，五项真实HTTP/合成RSA协议回归全部通过，0失败/0跳过。原独立reviewer仍暂不接受source1，确定两项P2：首次exchange失败后无Grant，公共接口不能沿用issued client重开授权；JWKS等待后才用now加expires_in会错误延长令牌寿命。五项通过不消除未覆盖缺陷，原冻结与finding保留。

Root已开始source2：增加不含凭据、可在exchange前保存的Registration元数据及正式重新授权入口，重开流程只用新PKCE/state/nonce/code，已有已验证subject仍必须匹配；token响应到达立即固定expiry时间锚，授权回调在全部await完成后再次检查本次事务deadline。新增首次失败恢复、verified subject防切换、迟到完成、access/refresh时间锚及refresh scope收窄反例。当前尚未冻结、重新独立接受或称回归通过。

本轮另仅对公开OpenID metadata/JWKS作无凭据GET，确有RS256与S256及公开token/revocation端点；metadata的一般scope列表不含direct scope，不能据此推断任何真实账号已经获授或该可选能力必然不可用。实际推理权限仍以签发token response为准，没有登录/刷新/模型请求。

匹配复核：有效进展是五项执行回执、独立两处根因和对应最小修正，不是更换目标或重复等待。原完整三向目标及源码/隔离边界不变；下一步只跑受影响协议回归并收原reviewer的delta，不重跑账号/网关/旧15请求，不安装或操作真实账号。

`chatgpt-plan-protocol-source2`已冻结：协议`0d12b7b83304ed7b8694692768f4901736b95005218eb8fe2d630643c5e51dd3`、八项测试`6844c754362d34f37409f1261b77bc07193824b05ebe4214d6ea94bbc2b0ca18`。原独立reviewer核对一致后给出**affected SOURCE/STATIC ACCEPT**，两项P2在源码层关闭，无新增finding；真实exchange/refresh路径确实使用receipt时间锚，finish检查在await之后，不是只添加无调用helper。时间边界回归明确是逻辑时间验证，不冒称HTTP延迟实测；fresh-code恢复和scope收窄仍经合成HTTP服务。

Root单job `cargo test --lib chatgpt_plan_auth --locked --offline` handle89753在途，尚无八项最终回执。编译器仍用已有1.96.1；首次误用未附带rustfmt的路径被拒，随后使用本机已有stable rustfmt完成格式化，没有安装或修改全局工具链。该路径问题不计作协议失败/通过；不重启已在编译的handle。

后继接缝范围保持明确：在现有CC授权中心和受保护应用数据存储中接provisional registration、显式授权/取消/清除、单账号refresh串行与await后代次检查，再由独立Qoder provider明确绑定具备实际direct scope的账号并遵守公开Responses限制。既有固定client Codex账户与默认项不被迁移或借用；Qoder账号SDK委派和原始模型供应商仍须分别验收，不因SDK结果文本可用就声明Codex底层模型已可切换。

handle89753已最终退出0：单job增量构建4分11秒，八项协议回归全部通过，0失败/0跳过，实际运行0.66秒。Root现接受source2的上述SOURCE/STATIC及OFFLINE范围，两处P2由修正、独立delta及回归共同收口；旧账号、网关、SDK及15请求未变证据未重跑。唯一dead-code警告是公开discovery尚未接入后继消费者，明确保留未接线状态，不将其消音当作完成。

本轮归类progress：有实际恢复入口、时间锚修正、独立接受和八项最终回执；无在途构建或待催作者。原完整目标继续active，下一必要包是受保护的计划账号管理及Qoder消费者，不以协议层交付代替它。没有安装/发布/提交/推送，没有读取真实登录缓存或进行账号/模型操作。最后一次文档追加曾因上下文尾句写错而被apply_patch完整拒绝，核对九行原尾后仅追加本回执，源码冻结未改。

### 2026-10-17 公开计划账号管理与授权中心接线

上一目标轮归类progress，八项协议回归和两项独立finding已实际收口。本轮Root新增`chatgpt-plan-accounts-source1`工作候选，原三向目标不缩减；这仍不是Qoder网关的计划供应商消费者，后者保持必需未完成。

- 复用CC授权中心、Tauri命令、opener和既有受保护应用目录，单独命名空间保存计划登记，旧Codex固定client账户不迁移。持久host先于打开浏览器；首次回调先持久issued client再单次exchange。Webview只得有限账号摘要及attempt/account ID，不返回授权URL或令牌。
- 使用短生命周期127.0.0.1 OAuth callback listener；不依赖代理服务正在运行，不新增常驻账号平台。nonce/PKCE/state继续由已接受协议负责，取消、到期或新登记代次后的迟到结果不能激活账号。正常载入只读状态，不自动登录/刷新/推理。
- 同一真实store的跨进程排他文件锁、单账号刷新锁、原子替换先于内存提交，未知刷新及被中断刷新不自动重放。退出与刷新串行以撤销最新replacement；保留登记、清除本地令牌，远端未确认与已确认分开。读摘要不发供应商请求、不改凭据。
- 新增六项实际loopback/真实磁盘回归：正向身份和无秘密视图、首次失败跨重启恢复、exchange在途取消及新尝试保护、并发单次刷新/最新token撤销、unknown跨重启不重发、落盘失败与中断恢复。尚无最终Rust回执，不能称离线通过。handle9565是唯一Rust单job，仍在编译，不重启或另起。
- 新授权中心界面三项Vitest已退出0，TypeScript检查随后退出0（同一handle23815已结束）。覆盖只读挂载/显式登录、身份授权不等于推理授权、unknown不可选择及自动观察本次回调，不把UI模拟当真实官方OAuth。既有baseline数据警告保留，未更新无关依赖。

上述候选已冻结并交原独立reviewer只读delta。核心SHA256：accounts实现`a72452876a8ba3595422bfc0e07a9e4896bdc242360c441955a927ba48470ace`；六项后端测试`4d81fa2853de473fb23f5efccd635cc9357b413a9ad5936198ecb732d3ee35c3`；Tauri命令`8cf2b940e34690171224316953855b1069c787f5f2ee09e5ea8706562506b27d`；UI`636bdd6f42fcf9f2ca22f709341c633569eb1dfb93498309e3449122a3c94587`。其余接线清单见同次派发，源码冻结不再并写；Root另标出“新登录取代在途refresh时phase收口”和“合成seed host一致性”待复核，未伪称无问题。

连续执行复核：本轮有生产登记管理/实际UI和三项执行证据，不是重复计划或空等待；当前具体待收只有handle9565与该独立候选结论。不改变真实账号、安装、目录或成本授权，不重复旧协议八项、原账号/SDK/网关/15请求。收口后继续明确绑定计划账号的Qoder消费者及官方模型目录/Responses限制，原始Qoder模型方向仍不以Agent委派替代。

#### accounts-source1执行结果、独立finding与负对照

原handle9565已退出0：增量5分37秒，六项实际HTTP/磁盘回归全部通过，0失败/0跳过，0.72秒；该回执不证明未覆盖边界。原reviewer SOURCE/STATIC暂不接受，确认一项P2：refresh派发后begin提升revision，随后取消或浏览器打开失败，旧refresh返回stale，但phase永久停在Refreshing。其余原包明确核对的身份隔离、先持久化、跨进程排他及未知不重试条件保留。

另两处证据不足按实情保留：合成seed的旧host不影响原刷新断言，但不能用于重登录交叉测试；原I/O失败只覆盖派发前保存，未覆盖服务器已轮换后的保存失败。Root新增真实token gate的两项负对照，并在合成OAuth时使用Manager真实生成的host，不在生产放松绑定；原grant(scope)继续用旧默认host，只有新专用测试走grant_for_host。当前仅改测试，source1生产逻辑尚未修正；单job handle25353只运行两项late_refresh，实际负对照结果待收，不把此前源码推断称已跑失败。

UI本次生产构建handle53349已退出0：SDK helper 4.07秒、renderer 3458模块28.92秒，沿用既有baseline/Browserslist/chunk警告，不为之改无关依赖。首次误选pnpm build（整包Tauri入口，handle69232）已由本任务Ctrl-C终止、退出1，未取得其构建成功回执或执行产品安装；现场未见该repo的Tauri子进程继续运行，随后仅使用正确build:renderer，不重启错误入口。格式检查只发现新增commands module顺序，已用已装rustfmt机械对齐，不改变逻辑。

最近有效进展是原六项实际结果、独立根因、真实新边界回归及生产renderer，不是重复等待；后继先消费25353负对照，再作根因修正并针对受影响delta复核。source1原冻结/finding和失败证据保留，生产账号、安装、外发授权不变。

#### accounts-source2负对照与受影响独立接受

原handle25353已退出1，实际编译3分23秒、两项运行0.35秒，均在预期状态断言失败（不是编译失败）：重授权取消后仍为Refreshing，以及服务器轮换后保存失败在当前进程仍为Refreshing。两项原始失败保留，不以随后成功覆盖。

Root最小修正位于共享管理器：begin持久化提升代次前将已派发refresh变为RefreshUnknown；refresh响应后的两处分支合并落盘，失败时立即将内存也变为RefreshUnknown，既不返回未保存的新token，也不重放旧refresh。派发前保存失败仍不冒称外部已经执行。磁盘保留的Refreshing重启后仍按原逻辑转unknown。

`chatgpt-plan-accounts-source2`三文件冻结SHA256：accounts实现`63bbfc91895909cd83eaa2735726a0d15540635097da7f2dde190e66b884a9bf`；accounts八项测试`39e0f0af9cdae2e8d16a8233e1c65e717c0a6dbcddf7b11ebe1185b0e7ebcb25`；协议合成fixture`9472155a2a924dd32c0caaa618eca07ce3f310c829798f4f585fea2eafcb7d50`。commands/mod.rs另仅机械排序，原协议生产逻辑、UI与命令不变，其接受和已有执行证据保留。

原reviewer已给出affected **SOURCE/STATIC ACCEPT**，原P2关闭、无新增finding；确认两项真实HTTP token gate、Manager host一致性、迟到结果和落盘失败后的no-replay，没有放松身份绑定。Root单job handle55022执行accounts八项，尚待最终回执，不将静态接受当测试通过。原三向目标仍active，尚未编写的Qoder计划供应商消费者与Qoder原始模型对外契约继续保留为必需，不安装或操作真实账号。

handle55022现已退出0：单job增量1分42秒，accounts八项全部通过，0失败/0跳过，实际运行0.59秒，含两项先失败后修正的新回归。Root接受source2的上述受影响SOURCE/STATIC及OFFLINE范围；源码冻结未变，无在途构建待轮询。原协议八项、SDK、旧账号/网关/15请求未重跑，UI/typecheck/renderer保持此前受影响证据，不称本次重新执行。

本轮归类progress：两个真实负对照、共享根因修正、独立接受与八项实际通过，非单纯状态循环。下一必要包为Qoder网关的公开计划供应商消费者：明确账号引用而非active/default回退，实际模型目录、实际scope、公开Responses请求限制与终态保留，以及原子身份快照、unknown不重放的生产router隔离回归。Qoder账号对外原始模型仍缺决定性契约，不用SDK完整Agent委派代替。原完整目标继续active，当前只授权源码/合成隔离验证，不安装发布、提交推送、读私有登录缓存或运行真实账号/模型。

### 2026-10-17 计划供应商消费者source1

上一轮有根因修复、负对照、独立接受及实际回归，归类progress。本轮Root仍为唯一作者，新增Qoder计划供应商消费者；原完整三向目标、隔离授权和未完成的Qoder原始模型方向不缩减。

- 现有Qoder provider配置增加显式planAccountId，计划分支仅允许公开固定OpenAI API Base、Responses及空API Key，不拿Codex当前账户/默认provider回填。旧显式Key配置与原Chat/Responses传输保持独立。
- 生产router固定本请求provider/账号引用后，沿PlanAccounts核对实际direct scope并取得token；用同一token读取当前账号的models目录，按visibility=list、原顺序及slug与配置子集核对，然后单次POST。使用既有代理/网络实现，不新建队列、调度器或账号平台。
- 公开计划要求的完整input、store=false/stream=true及不支持字段在派发前检查；支持的namespaced function/custom工具与输出、SSE终态原样转交，不用响应头、failed或中断冒称完成。当前明确拒绝平铺顶层function等未支持工具，不进行未经证实的自动协议改写；不能据本包声明所有Qoder工具请求格式已兼容。
- 授权中心复用既有Qoder网关表单和代理服务，明确账号、按用户点击读取实际目录，并能重新读取同页新登录的账号。挂载只读本地摘要，模型目录getter不是自动登录/推理入口。新的Tauri/API/invoke及中英文说明已接线，未启动真实应用或登录。
- 消费者断开可能取消等待token的future，因此public token_for增加owned task，既有串行refresh逻辑不变；一旦外发轮换仍由该任务持久收尾，不因消费者消失遗留活进程Refreshing。新HTTP gate回归同时观察旧unowned路径与owned路径，无自动重发。旧八项source2证据保留，非冒称它们已经覆盖新取消路径。

冻结候选`chatgpt-plan-consumer-source1`已交原独立reviewer只读SOURCE/STATIC复核，尚无接受回执。核心SHA256：inference模块`f2180b587da195ffa5580b9d27b8ef68625ba76fc442516d177f0fecc0e2bf90`；五项网关隔离回归`c6ea94c2b4a1909a8253e0ce7c461f19b1ba4353005964033430063f88d4e68b`；账号管理器`e485ed2a8c36927ffe7dad18c2bfadf0b1ecfee84140e16975c90c8290f262af`及其新取消回归文件`e4b7144aad0116f7624d1f83a0cfa0da07aec567ea79a7a0b964655ec9998b67`；生产router`8977ee55795b07c9192e2e4e25c8329c96430dfeadb2d7b98b950d19ccfecc49`；配置DAO`c46e82deb5b81aef679dcc34916d74ff2a5196aea8877781d39c3a14dfbacdef`；UI`b8b4e5a79e69410b735ae42b8dc7f15655aa0c28626af35463d06446d73dc3b7`；UI测试`9d7f2b60fea3a354bc89a8002dd417087cd8f6786806b616a5fcd9733d93f46f`。其他注册与接线随同次派发，本文状态尾可由Root追加，不改冻结源码。

实际前端：首次typecheck因重复JSX disabled属性TS17001退出1；修正后handle23589退出0。受影响QoderGateway界面五项handle73684全部通过（含显式账号而非active回填、显式目录读取与新授权后重读），0失败/0跳过。未重跑原账号UI、协议、SDK或旧15请求。

唯一Rust单job handle37899按五项新生产router/绑定回归、一项新消费者取消回归、受影响旧DAO三项及旧router七项顺序执行，尚待最终输出；不另启构建、不将编译中或静态复核当测试通过。当前没有安装、真实凭据/推理、发布或提交推送，原失败和未知记录保留。下一动作先消费本handle与原reviewer结论，再作受影响修正或接受，不能为通过改原目标。

#### consumer-source1实际回执与独立两项P2

handle37899已最终退出0：首次增量3分42秒，五项新生产router/配置回归全部通过，0.53秒；随后同一handle顺序执行一项新消费者取消回归（0.49秒）、受影响DAO三项（0.03秒）、旧router七项（0.05秒），均0失败/0跳过。旧未变协议八项/SDK/15请求未重跑。

原独立reviewer仍暂不接受source1，确认两项P2：模型目录GET等待后，账号退出/重授权换代/到期未重新核验，仍可新发旧token的推理POST；input中的additional_tools没有沿用工具前置政策，MCP/tool_search等可绕过顶层tools检查。上列通过不覆盖这两条路径，不以绿色结果替代必需修正。

Root开始source2负对照准备，source1实际逻辑尚未改：新增三个consumer_gap_测试，分别通过真实models HTTP gate完成合成退出/重授权后释放，以及additional_tools容器的允许项透传和七类已知禁用工具。退出仅调用同一sign_out_with到合成loopback discovery/revocation，方法仅扩大auth内部可见性，不改变生产控制流、不发送官方撤销。另移除本包未使用的测试import。handle37082是唯一Rust单job，仅跑上述三个新反例；尚无实际负对照终态，不重启/并行编译。UI构建另用既有build:renderer、handle10761，仅生成本地构建产物，不安装应用。

本轮有效进展是生产消费者及其真实HTTP证据、独立授权/工具容器根因，以及针对性反例；下一步先取37082，再用同一token的代次/有效期lease与共享工具政策最小修复。不得从全局active账号换token来绕过目录校验，也不得删掉function/custom的官方允许形式。总目标保持active，Qoder原始模型对外契约、安装和真实账号效果仍未完成。

#### consumer-source2负对照、修正及冻结

原handle37082实际退出1：增量2分42秒、运行0.36秒，三个新反例均在预期断言失败；退出/重授权目录gate后仍200而非409，additional_tools七种不支持工具均200而非400。不是编译失败，原记录保留。Root随后才修改实际逻辑，不用后续成功抹掉此证据。

source2引入的AccessLease仅是内部不可序列化的授权快照，不是新增用户权限凭据或自设有效期：固定owner、accountId、revision及同一token，匹配Connected和原Grant的真实scope/到期。目录读取全部await后及实际send_once前核验；变化明确409、dispatchStarted=false，不换token/账号重新过目录。新增到期回归是逻辑时间边界，不冒称真实HTTP等待令牌自然过期。

工具容器改用共享validate_tools：顶层保留已接受namespace/function/custom及web_search政策；additional_tools按官方developer角色接纳直接function/custom，同时拒绝同样的不支持类型，不改变input原有顺序、schema或内容。官方tools-tool-search正文L2387–2429已实际读取，采用其additional_tools格式，不引入该文其他tool_search能力；当前计划路由仍按preview-limitations禁止tool_search。

source2五文件冻结SHA256：inference`c46969ac3ff5d0532695203bf2496335ea417dd756adf5f5350df6c5387bb3a7`；gateway_tests`e9ffa66caad353fac518146486ce881b2a1cab35488a42f218ba81fb0ad332b5`；accounts`49177922ede65081aa92a4bdc85c32772ce565167b3a8fbe800b14028b3f0a46`；accounts_tests`ab074b9fa49016455ea053d3dd75dbc24d373a9e110370f99eceef7da406d544`；proxy/qoder`d60f79cb7ce3fb32eb556bc3ef2f28fd49997ba251b0bb8db000ba02a43f0945`。原独立reviewer已接affected只读复核，尚无接受。唯一Rust单job handle10539执行网关八项、新lease一项、受影响消费者取消一项，待最终回执；不轮询旧已结束37899/37082，不并发重建。

前端生产构建handle10761已退出0：SDK helper 3.89秒，renderer3458模块29.49秒，沿用baseline/Browserslist/duration/chunk等原非阻断警告，不更新无关依赖。UI与TS未因本次Rust修复改变，五项UI/typecheck/renderer证据保留，不重复运行。没有安装、发布、提交推送或真实账号操作。

#### consumer-source2实际回归与独立接受

唯一在途handle10539现已退出0：单job增量3分24秒，八项实际生产router/loopback网关回归全部通过（0.61秒）；随后同一handle执行lease真实授权快照的逻辑到期边界一项（0.21秒）、消费者取消后已派发refresh持久收尾一项（0.43秒），均0失败/0跳过。三个consumer_gap_负对照原先实际失败的记录保留，不将编译成功或模拟授权当作真实账号效果。旧SDK、协议、账号八项、旧网关、UI及15请求未变范围不重跑。

原独立reviewer已核对五文件冻结一致并给出affected **SOURCE/STATIC ACCEPT**：两项P2关闭、无新增finding。同一owner/account/revision/token在目录全部await之后和推理POST前复核，不自动换token；additional_tools与顶层工具共享前置检查，同时保留官方允许的function/custom定义和位置。内部lease不是新用户授权票据，逻辑时间回归也不冒称自然到期实机。Root接受上述SOURCE/STATIC及OFFLINE范围；当前无待轮询构建或reviewer，不把本包接受外推为三向目标完成。

目标仍active且不缩标：已形成Qoder SDK显式账号档案、独立BYOK网关和具备实际direct scope的ChatGPT计划供应商消费者的源码/隔离证据；Qoder账号对外原始模型契约及真实产品/账号效果尚未完成。下一动作只推进尚未决的原始模型供应商接缝或明确其决定性外部依赖，不重复已排除接口普查，不用完整Agent委派、最新prompt文本或SDK结果替代客户端工具历史/原始模型验收。当前仍禁止真实登录/刷新/计费推理、安装发布、提交推送和私有缓存读取。

本轮补充的定向公开证据：[CN Cloud Agents Quickstart](https://docs.qoder.cn/en/cloud-agents/quickstart)明确CN区域的openapi.qoder.com.cn与api.qoder.com.cn以及SAT交换需同区，但其业务示例是cloud环境、Agent、Session和events，scope为qca.access，另说明forward.access。该页解决的是云Agent区域/认证关系，不能据此将SDK认证页中的models.read/chat.completions示例拼成CN原始模型URL。未发送官方鉴权/推理请求，也未新增云Agent实现。部分SDK正文经浏览工具抓取失败，失败本身不证明功能不存在。

接续条件保持具体：Qoder账号对外模型方向需要官方确认的CN原始推理入口、相应个人/服务账号授权范围，以及完整消息历史、调用方工具往返和流式终态契约；现有SDK初始化、模型目录或完整Agent结果不能代证。真实客户端/账号验收另需用户明确对象与授权。本轮先收口已交付消费者，不为补足目标而猜私有端点、再次普查相同SDK接口或另造兼容层；收到决定性新契约后才继续该方向的最小源码实现。

### 2026-10-17 原始模型可行性收束及下游工具格式补差

上轮归类progress：consumer-source2十项实际通过及独立接受已消费，不再轮询旧handle。此前浏览工具失败的官方SDK认证与工作原理正文，本轮通过Node原生fetch从官方公开Markdown各取得HTTP200并完整读取；不是官方登录、token交换或模型请求。

复用现有独立reviewer只读裁决目标②，Root亦读取包1.0.53的`dist/protocol/common.d.ts`决定性类型：MessageParam.role仅为user。官方工作原理明确上下文、工具执行/拒绝结果、继续推理与压缩归CLI管理。reviewer据此判定现有公开契约不足以实现原标准的原始模型供应商，没有新发现的可直接实施路径，但不能推出官方宣布不支持。目标②停止同类接口扫描，等待官方CN原始模型契约或用户有效变更；原目标不缩标，SDK Agent/MCP委派仅为明确标注的替代，不抵原模型兼容验收。该独立复核未改文件/运行产品，实际后端型号无独立核验回执，不补造型号。

目标③尚有可独立推进的接缝，不能因目标②受阻停止整个目标：consumer-source2接受官方计划供应商格式，却拒绝普通Responses客户端的顶层function/custom定义。官方[Preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)允许通过additional_tools提供此类调用方工具；[工具格式说明](https://developers.openai.com/api/docs/guides/tools-tool-search)明确developer角色和按输入位置生效。后继仅做该已证明格式的最小整理，不吞掉历史/工具结果，不重命名工具，不接纳被禁止的宿主工具，也不重造协议服务。尚未取得Qoder客户端真实请求，不将合法兼容请求的隔离证据冒称原生实机。

Root先新增`consumer_gap_flat_client_tools_round_trip_without_losing_history`真实生产router/loopback负对照，覆盖function及custom各两轮、原已有additional_tools位置关系、保留的namespace/web_search、精确工具结果与混入MCP的前置拒绝。仅合成上游增补公开格式的回复，consumer-source2生产逻辑尚未改；当前唯一Rust单job只运行此反例，待实际失败后再修正，不将测试编写/编译当通过。无需重跑原SDK、协议、账号、UI或旧15请求。

#### consumer-source3负对照、最小修正及冻结

负对照handle13285现已退出1：实际增量2分14秒，运行0.32秒，新正向标准在400!=200处失败，证实source2拒绝该合法客户端格式；原失败保留。Root随后才修改生产路径，不用新成功覆盖旧结果。

source3只改三个文件：共享prepare_request先完成原输入/工具政策检查，再将顶层function/custom原定义按原顺序放进输入首项developer additional_tools，因为eager工具原先对全部上下文生效；原history/已有additional_tools相对位置、schema和调用名称保持。namespace/web_search留在顶层，不丢掉混入的禁用项以换取通过。只有明确绑定计划账号的生产caller重新编码该JSON并发送，原Key分支仍使用原body；账号scope、固定token的lease与禁止重试逻辑未变。没有新配置、依赖或用户授权步骤。旧flat拒绝用例由新双类型往返用例取代，负面矩阵保留明确不支持的file_search，不将缺参数类型当新原始要求。

冻结SHA256：inference`22ca375241f6b3ab2382a4fdb75134b9d919fd7804f4e6ee999f570d5488eb69`；proxy/qoder`99e7ef580527c8a1f1c9923ff892b78e764e53467baaf21e009e88ebda09ec87`；gateway_tests`7112f67d3350ea979bc127368d5ec2cb19b58d297acd0d203bb40e0df9cc964c`。已交同一原独立reviewer按该差异只读审阅，尚无接受。唯一Rust单job26659执行九项网关回归，待最终结果；不并发构建、不编辑在编译的Rust、不轮询已结束13285。格式检查及git diff --check已过，原Cargo.lock换行警告保留。此前未变UI/renderer/账号/协议/SDK及15请求证据不重跑。

#### source3九项实际通过及数值保真finding

handle26659已退出0：实际增量2分25秒，九项网关回归全部通过，运行1.42秒。独立reviewer仍判SOURCE/STATIC暂不接受，仅一项P2：整个计划正文的Value→to_vec会改变超大整数及高精度数值，包括本来不需整理的请求；原Value构造/捕获测试无法观察此损失。Root读取当前已生成serde_json fingerprint核实preserve_order/raw_value已有效、arbitrary_precision未启用，未另跑cargo探测，也不将九项绿色当作关闭该finding。

Root仅补测试与合成上游原始字节采集，source3生产逻辑暂不改。新`consumer_gap_plan_tool_preparation_preserves_raw_schema_numbers`以原始HTTP发送同一精确工具schema的flat及namespace两种形式，检查上游原始整数/小数及无需整理时的逐字节不变，禁止用Value断言自证。唯一单job65459只跑此新负对照，结果待收。后继拟复用现有serde_json RawValue做该分支局部整理，避免全局改变其他provider数值解析；先实际复现、再修正及冻结受影响差异，不并行修改在编译的Rust或重跑旧已过范围。

#### consumer-source4数值负对照与局部修正冻结

handle65459实际退出1：增量2分14秒、运行0.30秒，两种原始HTTP schema均丢失精确数值，断言为[false,false]而非[true,true]。该结果已证实source3 P2，原失败与其九项有限绿色证据共同保留。

Root随后只在计划消费者局部改用现有serde_json RawValue：Value继续作原政策检查，不用于重建用户正文；需要转换时从同一原始body借用字段、input与工具片段，仅重组数组和静态additional_tools项，工具原文、数值及历史不经浮点序列化。不需转换时返回None，caller保留原始body，Key分支亦保持原样。manifest只显式声明本构建已经有效启用的raw_value，无新增包/版本，也不全局开启arbitrary_precision/float_roundtrip；官方[RawValue文档](https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html)的保留原格式语义已实际读取，不使用unchecked构造。

四文件冻结SHA256：Cargo.toml`66cf2fc818459ab84da1942f7394bc7a89089333be0674d516dbb4fcc8c75f82`；inference`efa751b0e4c4783d23953d074f5f1736462be64d8cfa9992ab8a1fbb38033bfd`；proxy/qoder`95f0db43708b5b460948517a5aef7839fe3dc28af84c1d5607d9285999b2ea88`；gateway_tests`efed633e4395b3e4cf1772cd18328b4a4386616a424baaedaa92e8a10eceb1b9`。原独立reviewer已收affected只读delta，尚无接受；唯一Rust单job5859执行十项网关回归，尚待最终结果。格式及diff检查通过，原换行警告不作无关修复。旧未变范围不重跑，无产品/真实账号/安装/发布动作。

#### consumer-source4实际回归、接受与当前接续

handle5859现已最终退出0：单job增量2分55秒，十项实际生产router/loopback回归全部通过，运行1.62秒，0失败/0跳过。包括两类工具各两轮、精确原始数值、无需整理时的字节不变，以及原有目录/身份/授权换代和禁用工具反例。原source3数值失败及其有限通过回执保留，不以新结果改写旧事实。

同一原独立reviewer核对四文件冻结一致后给出affected **SOURCE/STATIC ACCEPT**，source3唯一P2关闭、无新增阻断；确认同一原始body与解析结果进入真实caller，只有Some才替换正文，RawValue保留工具和历史，无flat时原样。Root据实际回归与该独立结论接受SOURCE/STATIC及OFFLINE范围。当前无在途构建或待收reviewer；不再轮询5859或旧已结束任务。旧未变SDK/协议/账号/Key网关/UI/renderer/15请求只复用，不重复计数。

本轮归类progress：合法客户端工具的真实负对照、局部格式整理、独立数值finding、原始字节负对照、局部修正及十项最终回执均已交付。完整目标仍active、未完成：目标②仍缺官方确认的CN原始模型入口及对应完整历史/调用方工具/流式契约，现有SDK Agent委派不能替代；其他已通过源码/隔离范围不外推安装、真实授权或Qoder客户端实际消费。接续需上述决定性新官方材料或用户有效裁定；没有新材料不重复SDK/端点扫描、不猜测私有路由。当前源码阶段不自动申请或执行真实账号/计费推理，更不自动安装、发布、提交推送或关机。

#### 剩余阻断审计

source4收口后已连续三轮无可行动包：第一轮之前为progress，第一、第二、第三轮均为no progress，不将状态核对或本段登记计为实现进展。第三轮核对实际工作树、现有goal与接续回执，没有新的用户授权、决定性官方模型契约或待收交付；原构建与复核均已终结，已接受范围不重复测试/复核。必要未完成项仍是目标②，不能从SDK用户消息、MCP执行或SAT scope名推导原始模型兼容。替代的完整Agent委派可以明确另列，但不能代替原验收。

恢复条件：取得Qoder官方明确的CN原始模型URL与个人/服务账号适用授权，以及完整历史、调用方工具往返、流式终态契约；或收到用户有效的目标变更。当前没有这些输入，禁止以真实凭据试探、猜私有端点或通过安装/改账号来绕过。第三轮已满足同一真实阻断连续三轮的阈值，Root通过update_goal取得blocked回执，完整目标未标complete；停止无新依据的自动续查，下一次有效输入后再判。没有部署、提交推送、真实账号或付费模型动作。

### 2026-10-18 用户裁定后的①／③独立收口核对

用户明确保留②受阻且不接受Agent委派替代，要求核对①／③独立缺口并整理官方询问。此指令取代上一段对整个任务的停止接续；②仍不重新普查。真实账号、原生配置、安装、计费推理及提交推送的原限制未解除。

Root通过当前工作树及CodeGraph定位后，核对授权档案命令／SDK helper／授权中心、网关配置／实际router／UI，以及既有受影响接受记录。当前Tauri注册包含五项档案命令和网关配置／目录命令；现有HTTP server合入Qoder专属routes；SDK helper及LICENSE已纳入既有资源打包。没有把上述接线存在当作新的运行测试，也没有重跑旧通过用例。

| 方向／必需能力 | 已有适用证据 | 仍未完成与下一动作 |
| --- | --- | --- |
| ①显式授权档案、账号身份、选择／失效／解绑 | account-profiles-source2独立接受；Rust六项、构建helper三项及UI两项既有回执；本次当前调用链相符 | 真实PAT／指定CLI账号的核验与失效实机未做，需明确测试对象与真实账号授权后才执行；不是等待②的模型协议 |
| ①官方登录／退出／账号切换的能力边界 | 官方CLI文档确认交互式`/login`、`/logout`及显式配置目录；后继account-cli-entry-source2补齐从面板打开指定CLI的接线，已有独立接受及下述隔离证据 | 官方CLI中的登录／退出仍由用户操作与原生确认，不自动输入命令。OS终端GUI及真实登录未验收；桌面官方账号自动切换契约未证明，不将CC档案选择、解绑或CLI登录称作桌面换号 |
| ③第三方Key的独立Qoder网关 | qoder-gateway-source2独立接受、DAO／UI／实际router及SOCKS隔离证据；provider、模型、入站Key与上游身份分离 | 真实Qoder通过官方自定义模型界面连接该网关尚未验收；未安装CC候选，不能宣称客户端已可用 |
| ③具实际推理授权的ChatGPT计划供应商 | 独立授权协议／账号存储及consumer-source4接受；实际生产router合成HTTP十项通过；完整工具往返与原始数值保真 | 真实获授scope、真实模型目录和Qoder请求格式的兼容尚未实机验证。计划路径仅Responses，保留store=false／stream=true及前置拒绝要求；不能用普通Codex登录代替授权 |
| ③协议范围与其他产品不回归 | Chat Completions与Responses同协议转发、禁用格式前置拒绝和不回退Codex有既有证据 | Anthropic／跨协议转换当前不支持，不从其他CC路由外推；如果原生Qoder实际请求暴露新的必要差异，再按该证据补最小源码及回归 |

本次初核未发现既有已接受逻辑的新阻断；继续核对①实际使用入口后，发现从CC进入指定官方CLI账号界面尚缺接线，后继补差见下节。①真实登录／桌面切换和③安装／原生客户端效果仍明确未完成；它们不能由源码或合成回执代证。后续获准实机时仅使用指定隔离对象，先核对账号和配置范围，再做必要观察，不重放旧请求、不修改其他产品默认项。

②的待官方确认问题已独立整理为[Qoder CN官方协议咨询稿](qoder-cn-official-protocol-questions.md)，无本机路径、凭据、私人账号或会话材料。2026-10-18核实的CN官方文档和官网均列出`qoder@service.aliyun.com`；文档提供问题反馈及论坛入口。本次没有发邮件、提交工单或代用户公开提问。收到可判定的官方答复后，Root只重开答复实际解除的分支。

#### account-cli-entry-source1 独立补差与冻结

①的后继使用入口核对改变了下一动作：原先仅能在面板保存／核验档案，官方CLI操作必须另行手动寻找。Root现补一个「打开官方账号CLI」入口，仅面向明确的CLI目录档案；复用既有`misc::launch_terminal_running`，不扩展全局终端服务。启动所保存的CLI／可选Node与指定CN配置目录，清除四种继承PAT／Service Account环境变量，避免打开成环境中另一个账号。Windows用户路径只进入UTF16LE EncodedCommand，POSIX参数采用单引号引用；脚本不存令牌值。每次启动标签唯一，避免该既有helper的按label/PID脚本重名。

该入口不自动发送`/login`、`/logout`或模型prompt；由用户在官方交互界面操作并保留原生退出确认。只回显「请求打开终端」，不报已登录、切换桌面账号或核验通过；档案选择／身份历史不因打开而修改，之后仍需用户显式核验。PAT档案不走这个入口。真实应用/账号没有启动或改动。

新UI反例handle92048实际exit1，失败点为缺少入口按钮，旧两个用例因定向筛选跳过，不称业务失败。补实现后handle40932实际exit0，本文件三项通过，含仅显式点击启动准确ID、不自动核验/选择/保存/解绑和不报登录；typecheck handle55911退出0。旧SDK／网关／15请求未变不重跑。

后端新增两个`account_terminal_`回归：PAT前置拒绝／档案不变；通过生产命令生成器执行真实PowerShell或sh和合成Node CLI，覆盖中文／空格／单引号／百分号／感叹号／&路径、显式配置归属、四类环境凭据清除、无任务参数与合成配置sentinel不变。未调用真实Qoder或打开GUI终端。初次构建因未指定子目录Cargo.toml在编译前exit101；修正manifest参数后的唯一Rust单job39653仍在编译，不能称两个后端测试已通过，也不因观察间隔重启。

候选七文件冻结SHA256：`commands/qoder_auth.rs`为`01a36ce450baa3ec6545f123caec5e3021b70990ebd28116eede930891039bed`；`src-tauri/src/lib.rs`为`0f6a5152045ab7a2bc27fbcf1331bb8661c1ed89570406181f1ea7abfbc675c9`；API为`6feb21ccb0bd391b110c75c1b5e960a1d78fad8a3cc031091acc464771ddaadc`；UI为`f66c1a6b6176a8ba4b8161b44ac523706794dc9ac0cb93f4448f85eb4a8898c5`；en为`18252162dd0b0e3d215dde33ed232580896c38ea194b49d56a27d070d158035a`；zh为`c5348c97527484c6d0618bf0e12c7f1c9da6b1e30b1d0510bf6066d42e1fb2a0`；UI测试为`da434fe1b7f515b437195881e74a623dccc80d5477a618beceac074b17f9b6d8`。`git diff --check`通过。根交接文档为可追加状态，不属七文件冻结。

原子代理已不在当前执行树，Root已按原独立复核要求派发新只读角色`/root/qoder_account_cli_delta_review`，请求gpt-6.1-sol/max；不得将派发参数当实际模型回执。尚无接受，不安装或发布；③既有通过范围保留，②仍受阻不委派替代。下一步消费当前构建与该独审，再仅对必要差异修正、构建或收口。

#### account-cli-entry-source2 路径反例、修正与独立接受

独审对source1给出REVISE，唯一P2是Windows `.cmd/.bat`仍转交CMD执行：外层EncodedCommand不能阻止原字面路径的`%VAR%`被第二层解释。Root随后用两份只echo固定标记的合成batch实际复现：字面目录`%QODER_ACCOUNT_MARKER%`存在，但原PowerShell调用输出`wrong-expanded-cli`；同环境下采用单次环境变量展开且不用CALL，输出`literal-cli`并exit0。该记录是shell边界真实负对照，不冒称原Rust回归已执行。

为修此已证finding，Root通过原handle中止尚在依赖编译的39653，实际exit1、未进入测试；不是观察超时重启，也不是测试失败或通过。source2仅改Rust文件的Windows batch分支：路径先进入本PowerShell子进程的专用环境变量，再由`cmd /d /v:off /s /c`单次引用，不使用CALL；Node／exe路径及旧SDK核验不变。新增生产命令生成器的`.cmd`和`.bat`回归，覆盖可展开的`%VAR%`、decoy及中文／单引号／!／&路径，要求实际执行字面目标。之前两项回归保留。

新Rust冻结SHA256为`387a98ac84dd14cd878612e5922a34804404d4ea76915db5852e68dfe0192b35`，其余六文件与source1一致。原独审只复核受影响差异，已给出**affected SOURCE/STATIC ACCEPT**：P2关闭，无新增finding，未重跑测试或启动产品。Root保留已过UI三项／typecheck／格式检查证据；当前唯一单job20250复用既有缓存执行三个`account_terminal_`后端测试，仍待实际结果，尚不称本包OFFLINE通过。未触及真实账号、安装、发布或其他产品配置。

#### account-cli-entry-source2 实际结果与①／③当前终点

后端handle20250已最终exit0：`test` profile构建22分25秒，三个新增`account_terminal_`测试全部通过，运行3.78秒，0失败／0忽略；其他3760项是定向过滤，未运行，不计入通过。真实PowerShell执行生产生成的命令，分别验证PAT前置拒绝与档案不变、合成Node CLI的路径／目录／环境／无prompt归属、两种batch字面路径与decoy拒绝。该证据不覆盖既有OS终端启动器的GUI可见性，也不是实际Qoder登录。

随后只做受影响的`pnpm build:renderer`，handle2458最终exit0：SDK资源构建6.12秒，renderer3458模块34.39秒；沿用baseline／Browserslist／duration／混合导入和chunk大小警告，不更新无关依赖。前述UI三项、typecheck、冻结及独立SOURCE/STATIC接受仍适用。Root据同一候选接受本包SOURCE/STATIC与OFFLINE范围；两个handle均已终结，不再轮询，不做安装或发布。

①此阶段新增的可独立源码缺口已经补齐；档案管理与官方CLI入口不依赖②。仍缺指定对象的OS终端GUI、真实账号核验／登录退出／身份失效实机，以及没有已证公共契约的桌面账号自动切换。③原独立网关、Key／获授计划供应商和协议隔离的源码／离线接受保留，本轮没有新的已证必修源码缺口；仍缺隔离安装及真实Qoder通过官方自定义模型入口的目录／文本／流式／工具往返实机，不能称生产可用。②继续受阻、不以Agent委派替代。下一接续只按用户明确的安装／客户端／账号对象与授权进入对应实机，或消费官方协议答复；不为了已通过范围再造测试或继续接口普查。

### 2026-10-18 用户安排：保留受阻状态，转向DSH同步与供应商管理评估

用户要求先记录CC Switch／Qoder状态，再将DSH与远端同步并考虑由CC Switch管理DSH供应商。本段只调整下一工作面，不否定既有源码／离线接受，也不将待实机项统称为协议阻断。

- ②Qoder账号作为原始模型供应商：仍受官方CN推理、授权、历史／工具往返及流式契约缺失所阻。恢复条件沿用前述官方明确答复或用户有效变更目标；不以Agent委派替代，不重复端点探测。
- ①授权档案／指定官方CLI入口、③独立BYOK网关：已接受源码及隔离证据保留；安装、真实账号及原生Qoder客户端验收尚未执行，暂不推进。恢复需指定实机对象及对应授权，不能把源码通过称为客户端已可用。
- 本轮构建与独立复核均已结束，冻结候选和现有未提交改动保留；不重跑通过测试，不安装、不改真实账号或凭据，也不因DSH同步而提交或推送本CC工作树。
- 下一步先定位DSH仓库、核对本地与远端差异并安全同步；随后仅评估DSH供应商配置契约与现有CC Switch接点，不自动更改真实供应商配置。

#### DSH同步回执与供应商接入初评（2026-10-18，UTC+08:00）

DSH主仓库已确认为`<DSH_REPOSITORY>`，分支`master`。官方远端`origin`为`https://github.com/deepseek-ai/deepseek-harness.git`，用户远端`fork`为`https://github.com/WQMYH/ds-h.git`；没有操作旁边两份alpha checkout。同步前主仓库无暂存、未暂存或未跟踪改动；CC Switch的既有脏工作树独立保留，仅本交接文档追加状态，未随DSH同步上传。

| 对象 | 同步前已核实commit | 同步后commit |
| --- | --- | --- |
| DSH本地master | `c36a83ff6bb95e3f82cf79f9be7c724270a8aa61`（0.1.7-alpha.1） | `687f282e8625c6b8d11ff58211a90524be2e4317`（0.2.1-rc.2） |
| 官方origin/master | fetch后为`687f282e8625c6b8d11ff58211a90524be2e4317` | 同一commit；未向官方推送 |
| 用户fork/master | fetch后为`d347e70390`（旧本地缓存`4e84901e64`不冒充同步前实时远端） | `687f282e8625c6b8d11ff58211a90524be2e4317` |

本地和用户fork分别落后官方3037、7185个提交，均无独有提交。本地使用`git merge --ff-only`；用户fork使用原生`gh repo sync WQMYH/ds-h --source deepseek-ai/deepseek-harness --branch master`，实际exit0，未用force、rebase或新建commit。随后GitHub ref回读和重新fetch均确认本地／origin/master／fork/master三个完整OID一致，暂存和工作树仍空。同步只传入已有官方提交，没有上传本机文件、配置或凭据。

初次直接GitHub fetch分别连接重置／连接失败，未进入工作树更新；现场核实既有Mihomo mixed-port 7897后，仅对本次Git命令设置`http.proxy`、对本次gh进程设置HTTPS_PROXY，后续成功。未改全局代理／TUN。源码已更新不等于已安装或启动新版DSH；本轮未装依赖、构建、启动DSH或执行计费推理。官方该commit的check-runs查询为0项，不称CI通过。

用户另行允许仓库外CodeGraph索引。索引数据目录为`<EXTERNAL_CODEGRAPH_INDEX>`，仓库内仅`.codegraph` junction，并以本地`.git/info/exclude`排除；不提交索引或修改版本化忽略规则。旧HEAD初次索引实际通过（7264文件）；快进后的更新第一次与探索客户端重叠而返回database locked，原记录保留。探索进程结束后仅作一次串行sync，实际exit0：1492个变更文件（444新增／1048修改）、20946节点，1分3秒。该最终回执对应新HEAD；没有删索引、强杀其他进程或循环重试。

初评基于上述新HEAD的真实调用链，而不是安装列表或旧版本判断：

- DSH已有`llm-pi-ai`自定义供应商：`api`、`baseURL`、`models`、`apiKeyEnv`，支持`openai-completions`、`openai-responses`、`anthropic-messages`。配置与凭据按请求解析，修改供下一请求使用，不必重建Agent或会话。供应商ID被会话日志、默认模型与凭据引用使用，不应静默改名；已有会话保留自己的模型，不等于改供应商便切换所有会话。
- 原生`SettingsController.describe/mutate(ns, ops, expectedRevision)`提供脱敏读取、按路径修改和旧revision冲突拒绝；`CredentialsController.describe/set/unset`只返回配置状态，不返还密钥。现有UI确实调用两条路径。设置落在当前profile的`cordis.patch.yml`，Key由DSH本地凭据存储保存；不硬编码DSH_HOME、web profile或产品版本。
- `CustomProviderCard`先提交profile再保存Key，是两个独立动作：后者失败必须显示“配置已存、凭据未存”，只补缺失动作，不重复覆盖已提交profile。原生ConfigEditor协调profile/HMR写锁并保留无关YAML；CC不应整文件覆盖、直接编辑数据库或绕过revision。
- 当前CC Switch的`AppType`没有DSH，故尚非现成生产支持。已有Qoder网关仅覆盖Chat Completions／Responses，不能冒称DSH三协议已通，更不应让DSH误用Qoder命名空间或继承其账号选择。

建议首包只做CC Switch的独立DSH供应商配置适配：识别目标profile，导入／展示脱敏供应商，按稳定ID增改删受管配置，明确协议和模型能力，预览后写入并原生回读。优先复用DSH现有Settings／Credentials Remote，不另造模型执行器、队列、登录平台或协议转换；但CC作为外部进程的原生Remote连接／认证接线尚未验证，不能仅凭内部方法存在就宣称外部已可调用。必要离线patch导入也必须遵守原生写入协调，不能成为绕过路径。

必补证据：隔离的外部连接及profile识别；旧revision拒绝／保留其他供应商与未知字段；profile成功而凭据失败的部分状态；模型／协议／流式及工具往返的最小原生实测。首包可先限API Key供应商；DeepSeek原生账号登录／退出／OAuth刷新、聚合路由／自动故障切换及批量改已有会话模型是可选后续，不把它们作为配置接入前置。DSH现有账户和凭据本轮均未读取或操作；后续真实Key写入、账号动作和计费调用仍需相应授权。Qoder原始推理协议受阻不阻止这条独立适配工作。

本轮结论为同步完成和SOURCE接口可行性初评，不是DSH供应商适配实现／安装／真实推理验收；未重复先前Qoder已通过测试。下一步只有获准实施后才做上述最小配置适配和隔离回归。

### 2026-10-18 新active目标：DSH接入，随后Core原生适配

用户已明确授权观察CC其他智能体的接入配置，设计并持续完成DSH控制；DSH接入完成后，逐项考虑并完成Core对DSH、Kimi、Claude的适配。旧goal已清除，Root核实为空后建立新active目标。本节取代上一段“等待DSH实施授权”，不解除真实账号／凭据、未知重放或计费限制。

顺序：①以现有Pi／Hermes等供应商接线为参照，确定DSH原生配置／profile／凭据机制与最小接口；②完成CC端DSH配置控制、受影响回归与独立接受，再做隔离安装／实际读回；③先核现有Core原生接收与控制缺口，再按DSH、Kimi、Claude逐项补齐并验收原文读取、真实投递及关联回信。旧结果只回收不重测，不新增任务队列、登录平台或替代产品执行器。Root当前为CC／DSH唯一源码及集成写者；Core改动前须核现有工程包写集，不抢写。

Qoder状态保持完整：①档案管理／指定CLI入口SOURCE/OFFLINE已接受，GUI与真实账号核验／登录退出／失效未做，桌面自动换号无已证公共契约；②原始模型供应商仍受官方CN推理／授权／历史工具往返／流式契约缺失所阻，不以Agent委派替代；③独立BYOK及具实际推理授权的计划网关SOURCE/OFFLINE已接受，隔离安装与原生Qoder目录／文本／流式／工具消费仍未验。既有候选、失败记录与恢复条件保留；不因新目标恢复Qoder扫描、安装、账号动作或旧请求。

#### DSH配置控制source1实现接续

采用DSH原生HTTP Remote，而不是重写YAML、增加供应商执行器或把DSH强塞进全部AppType分支。CC现有授权中心新增独立DSH区块，显式连接正在运行的本机profile；只保存用户提供的原生连接地址。原生`/connect?token=…&interactions=none`交换Cookie，并保持拥有API Peer的mux WebSocket，随后调用`settings/describe`、带expectedRevision的`settings/mutate`和只写`credentials/set`。标准输出只含脱敏结果与原生rpcId；不读取已有Key、不复制浏览器Cookie，不新增自签票据。

供应商按固定ID编辑选定字段，保留其他原生字段和供应商；现有模型选项随ID保留，目录默认项可留空而不改写。新增Key使用独立原生引用，配置与凭据分别回执，未知写入不自动重试；删除供应商不删除可能共用的凭据。UI启动仅读取CC连接登记，不自动启动DSH或调用配置接口。连接地址输入与Key输入提交后清空，日志和UI不展示原生token。

截至本段：实际协议回归6项、UI回归2项和TypeScript检查已通过；原生Rust定向编译、renderer构建及官方DSH 0.2.1-rc.2的全新私有profile实测尚在进行，不记为安装或原生推理通过。测试只用合成供应商／凭据，不改变用户DSH_HOME、账号、默认模型、已有会话或计费配置。没有新依赖进入CC运行包；Node24原生fetch/WebSocket承担本机传输。

#### source2受影响修正与真实DSH配置回执

独立source1只读复核确认三项P2：刷新把旧草稿套入新revision、创建既有ID会更新原项、丢配置ACK时provider分步状态不准。Root先留失败回归，再修：草稿绑定开始编辑的revision；创建／更新意图贯通UI与helper并原生派发前拒绝冲突；配置派发前标记provider unknown，明确拒绝后才收敛。source2协议8项、UI原3项及typecheck通过；新增创建意图UI用例待回收。Rust定向1项与renderer已通过，未运行的3763项不计为通过；source2受影响独审与新renderer仍待收口。

官方发布的`@deepseek-ai/dsh@0.2.1-rc.2`仅安装在私有夹具（611个依赖，单次安装exit0，未全局安装）。run1在Windows批处理引号层启动失败，原生调用为0，不是产品投递未知或模型失败；修正夹具启动后用全新run2，保留run1，未重放任何既有请求。run2于2026-10-18T15:06:45.503Z至15:07:09.172Z通过原生配置验收：真实启动全新DSH_HOME／workspace的web profile、Cookie＋mux认证、读revision0、创建合成供应商／独立合成Key并获原生ACK、读revision1、旧revision拒绝且无mutate、删除供应商、读revision2并恢复原空列表。六次helper操作对应九笔原生RPC；原始rpcId与intent保留在私有run2/result.json及intent.jsonl。模型推理未运行、真实账号／凭据未使用、未知重放0。停止后现场只读进程核对未发现该夹具CLI残留；不把启动器signalCode尚未同步写入的null冒称产品仍运行。夹具后续补等待自身退出的记录，不为该日志改动重跑已过原生配置实验。

独立角色`/root/dsh_cc_source1_review`已对source2返回affected SOURCE/OFFLINE PASS，三finding逐项关闭、无新增阻断，完整核对五个受影响文件并独立读取run2原生RPC记录；未改文件或重跑实验。新增创建意图UI用例单项通过，另三项被过滤不重复计数；source2 renderer随后已实际exit0，3460模块。Root据原限定标准接受DSH配置控制SOURCE/OFFLINE和原生私有profile配置验证，不声称模型推理、正式客户端替换或Core DSH投递已通过。

为验证可见入口，构建独立标识`com.ccswitch.dsh-isolated`的同候选测试包，禁注册正式deep-link scheme，并使用`CC_SWITCH_TEST_HOME`避免真实账号／配置；包构建与UI入口观察在途。该隔离标识不回写源码配置，不将测试包冒充正式版本发布。后续Core接入先复用既有agent-mail native receiver；已只读核实原插件中台当前无活动源码写包，不重新派其已完成旧包。

隔离包已构建exit0（3分24秒），文件产品名实际为CC Switch DSH Isolated；只复制到私有`cc-source2`，未覆盖正式客户端。2026-10-18 23:18:13 +08启动的精确PID5892／测试路径已现场核实，实际SQLite、settings、日志均落入测试主目录。Computer Use按返回的唯一窗口7342218核对、关闭首次说明、打开授权中心，完整实际UIA显示`DSH · 原生供应商控制`及连接、读取、固定ID、协议、模型、Key、保存／新增入口；`dsh_native_status`路径显示“尚未配置DSH连接”，相应写操作禁用，无启动错误。未点击任何登录、账号、供应商切换或Key动作，不把入口观察当UI上真实连接／写入。原生配置写入由前述run2独立承担；两层覆盖分别保留。UI观察完成后按当前路径＋启动时间核对，只停止该闲置测试进程，测试数据和包保留。

DSH首个配置控制包的实现、回归、独审、隔离包入口和原生配置接口观察已收口；正式CC安装替换、用户真实DSH连接／Key、实际模型推理仍未执行，按用户边界保留。当前进入Core阶段：先复用既有Kimi／Claude native receiver和同一Mail队列，补DSH原生会话通知／读取及关联回信；不得把本配置包的9笔RPC作为Core投递证明。

#### 2026-10-19 Core阶段事故与当前停止边界

CC配置包前述SOURCE/OFFLINE、私有profile配置RPC和隔离入口证据保持，不曾通过CC修改真实账号或Key。后续独立Core包已安装agent-mail 0.2.1，DSH原生session身份核对／queue通知接纳通过；原文消费仍未通过，不能合并成完整接入完成。

Root随后在具体费用授权未核清时启动了使用既有DSH web profile的真实消费试验，这是操作失误。隔离的是workspace而非用户profile；产生8条主模型响应和1次标题模型请求，费用未知。原生启动将旧settings归档为settings.yaml.imported并导入现有profile，原件保留，未擅自回滚。旧源码也已包含该迁移，不能简单将其归因为本次升级。事故采集器收尾缺陷已与真实原生调用分别登记；真实模型／既有profile操作暂停，不通过再次启动或重放来补证。

完整脱敏证据／配置hash／非秘密字段名／恢复方案留在Core既有主计划与Git忽略的本机事故回执，未公开上传。当前无该测试进程／监听，私有Core登记已禁用，正式DSH receiver未配置；原文、native intent和待审批历史保留。保留迁移还是按指定旧入口恢复，须用户确认后单独执行；缺少迁移前patch的preimage，不能自动删除或覆盖它。Qoder原三项已过／受阻／恢复条件保持本文件前述完整登记，本事故不触发Qoder、账号接入或旧业务重启。

### 2026-10-19 CC 更新与 DSH 原生模型聚合接点

- 同一中午DSH会话现已完成真实取信、关联回信及双ACK；旧“未取信”是历史停点。回信一个汉字不准确仍保留，不重跑邮件测试。用户随后转回CC／DSH模型配置，要求以CC实际控制方式和DSH原生源码为准，不另造模型管理。
- 当前兼容分支已从797e0276ffc20e54cbdf55e3de84505d77231c91快进官方upstream/main的36提交，HEAD为adc223af322adaeca878e82bf78323865b0db768／4.0.15。9个重叠源码路径先留可恢复stash 78a3de1652780de71386a61eaf0f45166c39f041，随后apply无冲突；原暂存5文件及其余兼容改动保留。stash不删，未commit／push。首次直连GitHub失败；仅当次Git使用现场核实的本机代理，无全局代理／TUN变化。
- CC直连改原生配置，路由／Stack才走代理。现有Stack面向Claude／Codex系列，不将DSH冒充另一应用。DSH已有多provider并存、按provider/model解析和原生统一目录；复用既有设置／凭据接口，新增只读model_catalog到session/modelCatalog，不增加模型管理器、网关、队列或推理执行器。
- CC的DSH区块显式读取原生聚合目录，显示供应商／模型身份、思考档位、原生默认和单供应商不可用。同名模型不跨供应商合并；设置读写后清除旧目录，目录读取不解除unknown写hold。仅输出字段白名单，不返原生失败异常中的可能敏感正文；官方账号仅列模型，不混作pi-ai表单的账号管理。
- 受影响协议10项／UI6项、typecheck、renderer均exit0，renderer3471模块；内存负控删除入口后成功oracle拒绝，0网络调用且未改生产文件。生成helper与源同SHA。Rust定向首次因普通shell缺link.exe而未进入测试，已仅用既有VS进程级开发环境续编；handle77433未回收终态前不称Rust通过或已安装。
- 现有DSH只读实机：13:42:39 +08的session/modelCatalog accepted，实际DeepSeek一组、flash／v4-pro两个模型及off／low／high／max；13:45:35 +08的settings/describe accepted，llm-pi-ai revision0、writable=true、自定义provider0。两次writeState均not_attempted，未创建会话、发信、写配置／凭据或调用模型。当前接线回执为.runtime/dsh-native-aggregate-20261019/RESULT.json，通用使用说明为compat/dsh/README.md。
- 原配置source2的独立接受只覆盖旧范围；新增目录增量尚未独立复核，正式CC替换及真实多供应商推理未执行，不把只读目录当聚合推理验收。

- Rust的77433已回收exit101：应用lib test的rustc-LLVM报out of memory／Allocation failed，Windows进程退出0xc0000409，未进入断言。保留该失败，不以重新运行改源码取绿；已基于明确内存原因，仅对本cc-switch包关闭test调试符号并改单job，复用全部已编依赖作一次有界接续，不改测试选择、代码、安全设置或其他应用。若相同资源失败复现则停止该构建支线，不继续试参循环。

#### 2026-10-19 原生验收接续、独审修正与并排运行

此前低内存handle3779最终因LLVM输出空间耗尽exit101，未进入断言；E盘当时为0可用，旧本机RESULT文件写入失败变为0字节，完整失败与已有证据已恢复在D盘临时目录。旧OOM／磁盘失败保留，不抹成测试失败或通过。用户随后明确授权资源恢复后接续原生编译、独审及不覆盖既有安装的运行，并允许数据进入候选；不是恢复模型调用、账号／Key操作或旧消息重放。

原生定向handle96126使用既有VS进程级环境、单job和仅本包test debug=0，6分48秒构建后exit0：dsh_connection_metadata_never_exposes_token实际1项通过，0失败，3824项过滤未运行。未重跑已过的10项协议、6项UI或typecheck。

独立角色/root/cc_dsh_catalog_independent_review核对冻结文件，新增目录接线／脱敏／供应商身份／默认与思考档位无新增阻断，但确认P2：带value的设置inspect可解除先前unknown写hold。Root新增“凭据写入未知→设置刷新”的单项回归，旧代码准确失败于保存按钮被解锁；随后仅删回复处理条件中的reply.value ||。定向绿回执1 passed／6 skipped，保存和删除继续禁用、unknown提示保留、原生调用次数不增加。原复核者对两文件新冻结值返回affected SOURCE/STATIC PASS，关闭该finding；未实现者复核，未由其修改或重跑测试。因实际UI代码变化，仅重建renderer产物，3471模块、exit0；不为取绿重跑其他已过测试。

Root使用源码外Tauri overlay构建独立标识com.ccswitch.dsh-native-verification／CC Switch DSH Preview，无deep-link schemes，无MSI／NSIS，beforeBuildCommand留空复用新renderer。handle52952原生独立构建5分53秒exit0。并排副本于16:00:38 +08启动，PID61828；使用新的CC验证home与无旧Store override的AppData，未复制共享数据库／账号／密钥，也未创建新的DSH profile。源码安装配置未回写。正式`<CC_SWITCH_INSTALL>/cc-switch.exe`在运行前后SHA256均为8d10b678f70a4e064feed84c77c91fd14f64093bb9a8f78f7e8c5a9cc9b031bd，未覆盖原安装。

实际Windows窗口已出现，左侧授权中心的真实页面文档包含“DSH · 原生供应商控制”、原生连接、设置读取、聚合目录、固定供应商ID／协议／模型与保存／新增控件；当前“尚未配置DSH连接”，未在此UI上重复既有DSH实机实验或执行写入。用户指出应用里找不到DSH：原因是目前只把DshProvidersSection接在AuthCenterPanel底部，没有注册应用管理条目或独立主导航，因此不宣称完整应用级接入。此入口／导航缺口保留，不用构建、独审或隐藏功能区的存在充当整体体验已完成。

当前可恢复接续记录位于文枢本机.runtime/cc-dsh-native-aggregate-20261019/RESULT.json及REVIEW-BRIEF.md，包含原失败、红绿日志引用、冻结值和独立运行身份。未提交／推送、未改真实供应商配置／账号／API Key、未调用模型或重放消息。Qoder原①／②／③的已过、受阻与恢复条件保持前文，不因本包重新测试或恢复账号操作。此debug运行副本仍依赖本地checkout的helper查找路径，不作为可跨机器发布包。

#### 用户纠正产品目标：应用级接入与既有账号流向

用户进一步明确：Qoder和DSH均应加入CC现有“应用”体系，复用应用管理与主入口，不是继续在授权中心堆设置；Qoder CLI路径应由既有发现机制自动识别，不能常态要求用户手填绝对路径。Codex接入方向是复用CC已经管理的Codex账号，经既有供应商／模型机制进入Qoder模型列表，不应再做第二套专属ChatGPT登录、账号池或凭据复制。

Root承认此前将完整应用接入缩成局部设置表单，产品边界和账号复用方向理解有偏差。前述协议、原生配置、未知保护、编译、独审及独立运行结果保留，但只证明相应底层／局部功能，不能作为用户完整要求已经交付的依据。当前候选回退到“可复用局部基线”，不继续沿错误布局增加设置，不覆盖正式安装、切换真实账号、导入密钥或触发模型。

正确后续范围：核对并复用CC应用登记／应用管理／供应商与模型列表的现有调用链；将Qoder和DSH按真实支持能力接入，原生DSH设置／目录接口继续复用；Qoder CLI自动发现沿用既有机制，必要手动指定只作实际未发现时的明确例外；Codex账号选择、引用和供应商凭据解析复用CC既有管理，不额外实现账号系统。旧②“Qoder官方账号作为外部原始推理供应商”的协议受阻，与本次“CC托管Codex账号进入Qoder模型列表”不是同一方向，不能互相替代或套用阻断。先前付费／真实账号／未知请求边界保持，不因产品纠正扩权。

#### 应用级预览纠正（用户初验优先，2026-10-19）

Qoder CN 与 DSH 已登记到现有 AppId／可见应用／Apps 安装管理和侧栏主导航，供应商页复用各自已存在的原生接口。授权中心移除两者控制表单及额外计划账号登录入口，仅保留 CC 原生账号管理；Qoder 应用页从该既有 Codex 账号目录选择明确账号，并复用 CodexOAuthManager 与已有模型目录服务，不复制账号或自动迁移旧 planAccountId。Qoder CLI 自动检测复用有效 PATH 和现有安装搜索目录，不绑定产品版本路径；手填路径仅作为例外。未支持的通用供应商投影、MCP、Skills 和配置目录入口明确隔离，不通过新增应用 ID 伪造支持。

供应商以列表与添加／编辑弹窗展示；DSH 原生连接单独折叠，读取不解除 unknown 写入保护。新增应用能力边界及 CC 账号引用回归已编写，按用户要求不在初验之前执行后续回归／独审或真实模型验收。TypeScript 与 renderer 构建通过，仅为生成可打开的独立预览，不等于功能验收。原生构建 handle33317 后已完成 exit0，并打开过该版预览；用户再次指出其独立表单、空数据与手工连接不符合需求，因此该版不能作为产品初验通过。旧已过与失败证据均保留。

#### 2026-10-19 返工：直接使用 CC 托管供应商（用户初验前）

用户明确取消手动路径／连接地址兜底：无论未发现、多份安装或检测异常，均进入现有 CC「应用」安装／升级／删除流程，由用户明确选择；不自动安装或清理实例。该要求覆盖此前“手填作为例外”的提议。

Qoder／DSH 主页面已撤下 QoderGatewaySection、QoderAuthSection、DshProvidersSection 独立表单，改用同一个 ManagedNativeApplication 接线组件，直接消费 CC 已有 useProvidersQuery、useProviderActions、ProviderList、AddProviderDialog、EditProviderDialog。Qoder 读取现有 Codex 供应商与其托管账号引用；DSH 可在现有 Codex／Claude 供应商来源间选择，不复制 API Key，不新建账号池，不要求重新登录。适配目标只存 sourceApp/providerId 引用，选择引用本身不等于已写入 Qoder／DSH 原生配置，也不宣称聚合推理通过。

CLI 定位使用现有 locate_default_tool（有效 PATH、原搜索目录、默认安装歧义判断）；数据目录按 DSH_HOME／QODERCN_CONFIG_DIR 和官方默认目录自动计算，无版本硬编码。异常只导向现有 AppsPage，无新的手工路径或 token 输入框。

为在不修改真实账号／配置的前提下展示现有托管数据，debug 初验支持 CC_SWITCH_READONLY_PREVIEW=1：打开原 CC 数据库的只读连接，载入既有账号元信息，不使用空 CC_SWITCH_TEST_HOME；跳过正常启动迁移、原生配置接管、后台同步与退出清理。初验 IPC 仅开放必要读取和检测，保存／认证／token 刷新／安装删除／模型探测均不执行。只读预览不是新认证机制，不改变正式安装行为。

handle87607 类型检查和前端构建 exit0，3464 模块、52.80秒；必要原生构建 handle99414 在原独立应用标识下进行，不运行安装器。新增针对既有目录／窗口复用、无手工表单、只读初验不派发变更与源引用校验的回归已准备，按用户初验优先要求暂不运行。正式程序、真实账号、既有未知消息保持原样；本轮模型调用为零。

用户随后限定只做 Qoder／DSH。临时针对其他账号模块的预览防护改动已撤回且差异为零，防护收敛到当前只读预览入口：只允许这两页所需的 Codex 账号元信息，不进入其他账号的迁移／认证路径。首轮 E0282 编译失败已保留，随后补上 Tauri handler 的具体运行时类型；最终限定范围构建 handle83301 exit0，1分31秒。未覆盖正式安装，打开独立只读副本，未复制数据库、账号或凭据。

实际 UI 已观察到 Qoder／DSH 侧栏入口，DSH 自动识别 CLI 和默认数据目录，页面展示原 CC 库中的四张供应商卡片及 CC 原生预设供应商窗口，不再出现独立路径／连接地址／密钥表单。正式程序及三份账号文件的启动前后指纹相同，原数据库大小与修改时间不变。只读预览会阻止原卡片自动额度查询、配置写入和模型调用，因此可能显示对应只读提示；这不作为供应商可用性结论。

停点为用户界面初验：尚未执行新增回归、当前增量独立复核、Qoder 检测页面单独实测、原生配置写入或真实模型／聚合推理验收。关联既有供应商引用与成功写入原生客户端配置仍是不同状态，不把当前界面展示升级为生产控制面完整通过。

用户截图再次纠正布局：卡片与弹窗复用不足以保留专用供应商页面。已删除 Qoder／DSH 页的说明框、CLI／数据目录展示、重复添加／安装／打开库按钮、来源页签与过程提示；添加和搜索接回原 AppPageHeader，安装仍走原应用菜单，DSH 来源选择仅置于既有应用更多操作菜单。主区域沿用普通供应商页的滚动容器与 ProviderList，原新增／编辑窗口由该共用页头控制，根层不再为这两个目标同时挂载第二份错误应用类型的窗口。仅改两页接线及相应待执行回归，不改其他账号模块；typecheck 与必要前端构建 handle76900 exit0，33.19秒。重新生成并排候选后仍停在用户初验，不执行配置写入、模型调用或后续验收。
