# 个人仓库 Windows 发布

这个分支的 Windows x64 更新源为 `WQMYH/multi-compat-cc` 的正式 Release。提交到 `main` 会触发打包；手动运行同一工作流只生成限期保存的构建产物。推送与应用版本完全一致的 `v` 标签时，工作流确认该提交属于 `main`，然后发布正式版本。其他仓库、分支和外部 PR 不会获得发布签名密钥。

发布产物包括 MSI、带 `portable.ini` 的便携 ZIP、更新清单及 SHA-256 清单。工作流使用仓库中的锁文件，并使用该仓库专用的 Tauri updater 密钥签名；它会用程序内公钥验证签名、版本绑定、篡改和错误公钥。私钥和密码只保存在受保护的位置与 GitHub Actions 加密 secrets 中，不放入源码或打包产物。Tauri 更新签名不是 Windows Authenticode 代码签名。

构建使用 GitHub 托管的 Windows x64 临时环境、Node 24.22.1、pnpm 10.12.3、Rust 1.95.0 和冻结的依赖锁；运行记录包含镜像、工具版本和锁文件哈希。Tauri CLI 2.12.4 使用 [WiX 3.14.1 官方发布包](https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip)，并校验其 SHA-256 `6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31`。如目标电脑没有 WebView2，MSI 会按 Tauri 默认方式从 Microsoft 下载所需运行时。上游仓库专用的定时、模型、同步及其他平台工作流在个人仓库不执行。

新版本需要同步更新 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 中的应用包版本和 `src-tauri/tauri.conf.json`，运行检查并提交后，再从该提交创建匹配的 `v<版本>` 标签。发布清单的地址始终指向此仓库，不回退到官方镜像。

没有匹配 MSI 注册的手工放置 EXE 和带 `portable.ini` 的便携 ZIP 使用发布页获取新版本；它们不会被当成 MSI 安装来替换。只有本项目 MSI 注册路径与当前程序匹配时才允许调用 MSI 更新器。如果直接把手工 EXE 覆盖到原 MSI 安装目录，旧注册可能仍然存在；要明确改用便携模式，应先正常卸载原 MSI，或在 EXE 同目录保留官方的 `portable.ini` 标记。已安装的旧程序仍包含旧更新公钥和地址，首次切换到本仓库需要安装一次带新公钥的版本。

MSI 允许选择程序目录，默认使用当前用户可写的程序目录。程序默认将应用数据和缓存放在 EXE 所在目录下的 `data` 和 `cache`，目录不可写时报告错误，不回退到其他位置。启动时可用 `--data-dir` 指定数据根目录，并可用 `--cache-dir` 指定独立缓存根目录；只指定数据目录时，缓存默认放在该目录的 `cache` 子目录中。例如 `cc-switch.exe --data-dir "D:\CC Switch Data"`。已经启用开机自启的用户修改这些参数后，应在设置中重新启用自启，以更新启动命令；自动重启会保留原有参数。可以通过参数继续原位使用已有数据目录。首次发现其他旧的 CC Switch 数据时会询问是否迁移；保留旧数据，不覆盖冲突文件。

Windows Installer 仍可能在系统盘维护其自身的安装数据库缓存；Tauri 当前 MSI 更新器也可能在应用缓存中留下临时安装器目录，不能仅凭文件名判断其是否可以安全删除。平铺 EXE 的受管理更新不走这个 MSI 临时安装流程。
