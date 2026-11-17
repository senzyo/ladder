# 2026-11-17 验收记录

本文保留 2026-11-17 前序实现阶段的检查记录，并在末尾记录本次交接核对。历史测试和构建不能代表后续代码；没有对应提交或证据的项目不补推为通过。ladder-main 只作参考，其原有 README 改动未触碰。

## 前序实现阶段检查（历史记录）

- Rust 全目标编译、rustfmt 与 clippy（`-D warnings`）。第三方 `proc-macro-error2` 有 Cargo 的未来编译兼容提醒，不属于当前编译失败。
- 安全回归共 77 项通过：主集合 74 项，加独立配置列表及验证运行测试 3 项；涵盖配置/TUN 副本、日志脱敏、DNS 模拟、GitHub 前缀、下载与归档检查、更新器版本/架构/资产校验。
- 新增配置验证测试使用唯一名称的无害模拟 EXE，覆盖三种核心命令、错误输出脱敏和取消；锁定配置的错误在刷新后仍保留。
- mihomo 的 YAML 合并键/TUN 副本、JSON/YAML 列表与独立选择、v1/compatible 资产、Alpha 版本和归档 EXE 归一化。
- 发布脚本实际打包，核对 ZIP 白名单、相同输入的一致性及 SHA-256；修复真实 MSVC 环境初始化中发现的命令引用问题。
- `3.0.0-alpha.1` 的 amd64 与 arm64 Release 均实际构建成功，两个便携包及对应 SHA-256 已生成；最终包再次核对文件白名单和校验和通过。
- Windows PowerShell 5.1 更新助手实际执行无害 EXE 的替换、重启、失败回滚、无提交标记取消和设置保留；Rust 持有者销毁会终止未提交助手。

## 界面检查

Computer Use 操作独立 `Ladder TODO 隔离验收` 窗口，生产 UI 文件原样复制并记录哈希，运行/日志/更新均为内存 stub。窗口为 900×600 逻辑像素（截图 125% 缩放），未编入真实 DNS、TUN、核心进程、恢复、网络或安装模块。

已检查：应用图标与标题、三核心导航、当前配置验证、批量结果标签与行下错误、浅/深色主题、版本列表及前缀快捷填充/全选/文字输入/Enter 保存、关于页检查/下载/准备安装状态，以及 mihomo 日志切换。实测发现 DNS 长提示把开关挤出窗口，修复后复测确认提示换行且开关可见。Alpha 版本文字随后改为不额外添加 `v`。

本机证据与安全模块清单保存在忽略目录 `target/todo-ui-check/`，不加入发行包或 Git。

## Review 修复

- 不可读配置的验证错误不再被定时扫描清除；批量检查按项处理失败。
- 取消批次会清理未完成状态；配置/核心文件改变后失效旧结果。
- API 与下载共用经过校验的加速前缀；不输出前缀中的凭据。
- 所有进程失败提示移除 exit code，保留可操作的错误说明。
- 软件更新助手转到后台就绪，并在退出成功后才提交安装；退出失败不会遗留仍等待安装的助手。
- 本地发布校验标签与 HEAD、版本及架构，拒绝跳过构建发布。
- 核对现有正式发行版为 `2.34.0` 后，将重写开发版版本设为 `3.0.0-alpha.1`，避免将旧实现当成升级目标；版本比较回归确认跳过旧版并接受未来 `3.0.0` 正式版。
- 代码清理审核未发现其他可确认删除的生产死代码；移除了多余的测试状态改动，保留有实际调用的恢复路径。

## 验收边界

前序实现阶段未在本机启动真实 Ladder 或真实核心、修改 DNS/TUN、注册系统任务，也未运行会创建同名核心的完整进程测试。新工作流定义了在隔离 runner 执行完整测试的作业，当时未实际触发；真实核心/TUN/DNS/托盘和 ARM64 机器启动仍需发布前隔离验收。

前序实现阶段只验证本地构建与工作流定义，未推送代码或标签、未运行远端 Actions、未创建 GitHub Release。旧正式版只用于更新目标过滤；完整联网自升级须用本次重写实现的前后两个版本验证。

## 后续界面调整验收

- 当前配置增加“编辑”；导航、设置、日志和托盘统一以 mihomo 为首项，默认进入 mihomo。
- “关于”中的 Ladder、mihomo、sing-box、Xray 显示仓库名称链接。实测鼠标打开 Ladder 仓库、Tab 聚焦后按 Enter 打开 mihomo 仓库；其余链接地址及共用行为经代码审查核对。
- 移除 gh-proxy 按钮。空输入显示灰色 URL；实测 Tab 补全并保存、手动输入后 Tab 不覆盖、Enter 保存、清空后保存直连、空输入 Shift+Tab 不补全。提示文字本身不写入设置。
- 复用纯 UI harness，四个生产 UI 文件逐字同步并核对哈希；在 900×600 逻辑像素下检查浅/深色主题、当前配置编辑路径和日志 Tab 顺序。编辑与设置保存使用内存 stub 和固定结果标签，无真实核心、DNS、TUN 或安装操作。
- `cargo fmt --all -- --check`、`cargo check --locked`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo build --locked` 通过；安全回归集合 74 项通过。主程序仅构建，未启动。
- 规范与需求两条独立审查均通过。已修复日志显示顺序与 Tab 顺序不一致，以及前次保存未完成时可能丢弃 Tab 保存请求的问题。
- 本机证据位于忽略目录 `target/todo-ui-check/evidence/followup-*`；验收程序和临时打开的仓库页面均已关闭。

## 关于页更新操作调整

- 四个仓库链接去除下划线；更新操作合并为最右侧单个按钮，按状态显示“检查更新”“下载更新”“重启以更新”。进行中禁用，进度和版本状态位于链接右侧。
- 使用同步生产 UI 的内存 harness，在 900×600 逻辑像素、完整 `3.0.0-alpha.1` 版本文本下验收浅/深色布局。模拟检查、下载进度、准备完成、重启中、失败后键盘重试及已是最新版本，按钮位置保持稳定。未执行真实更新、核心或系统网络操作。
- `cargo fmt --all -- --check`、`cargo check --locked`、`cargo clippy --locked --all-targets -- -D warnings`、harness 与生产构建通过。规范和需求独立审查均无发现。
- 本轮证据保存在 `target/todo-ui-check/evidence/about-update-*`，验收窗口已关闭。

## 交接核对：图标、间距和对齐证据补记

本次接手基准为 `7102fddd961cda6dc308fc714163d73cd86fb074`，程序版本 `3.0.0-alpha.1`。以下是复核既有纯 UI harness 证据，不是本轮重新启动真实 Ladder 的验收。

- 发现并核对 `target/todo-ui-check/evidence/about-spacing-source-manifest.json` 和 `about-spacing-ui-hash-check.json`；`ui.rs`、`proxy_input.rs`、`log_view.rs`、`scrollbar.rs` 四个生产文件与记录中的 SHA-256 一致。
- 复看 `about-spacing-light.png`、`about-spacing-dark.png`，图标与标题、版本/MIT 同行、关于区更新提示和按钮对齐正常。目录另有 `about-spacing-available.png` 和 `about-spacing-progress.png`，保留为该次状态截图。
- 同目录的 `about-spacing-cargo-check.log`、`about-spacing-clippy.log`、`about-spacing-production-build.log` 和 `about-spacing-harness-build.log` 均记录成功；这是当时的检查和开发构建日志，不代替本轮修改后的检查或最终双架构发行构建。
- harness 清单明确排除真实 Runtime、DNS、TUN、恢复、任务计划、核心进程、网络更新、更新助手和系统托盘；截图只能支持界面布局结论。

## 交接核对：全仓死代码审查

静态审查覆盖 `git ls-files` 中的全部 Rust 模块、生产函数及事件入口、16 个直接运行依赖和构建依赖、Windows 特性、三个发布/更新脚本、嵌入权限脚本、资产和测试 fixture。使用模块图、名称引用搜索、调用链阅读及 `cargo tree --locked -e features -i windows-sys@0.61.2` 交叉核对，未用 clippy 无警告替代审查。

- 未发现可确认删除的生产模块、函数或状态分支。全部 Rust 源文件从 `main.rs`、`lib.rs` 或 `ui.rs` 进入构建图；图标、manifest、权限脚本和更新助手均有嵌入或工作流引用。
- `Runtime::open_with_dns`、正常停止/退出和异常退出仍调用 DNS 恢复；`lib.rs` 的两个专用启动参数分别进入 panic 和开机恢复；`scheduler::desktop_dns` 仍使用机器级/便携 lease 及 legacy journal 接续。保留这些路径。
- 测试专用的 `--exact` 自启动、fixture 的 `--child` 和环境变量分支仍用于进程树/panic 测试，不属于废弃分支。`fmt::Write::write_str` 等 trait 入口也不能按普通函数引用次数删除。
- `windows-sys/Win32_System_Registry` 曾因无直接 Registry 模块调用被列为清理候选；实际删除后，`cargo test --no-run` 报 E0432：`desktop.rs` 使用的 `SHELLEXECUTEINFOW` 和 `ShellExecuteExW` 受该特性控制。候选经编译证伪，已恢复。API 的跨模块特性依赖不能仅靠直接名称搜索或聚合依赖树判断；本轮没有删除依赖声明。

## 本轮交接状态

核对环境：主机 Windows 10 22H2、PowerShell 7；真实核心和完整进程套件在 AMD64 Windows 沙盒执行，沙盒 PowerShell 为 `5.1.19041.6456`。接手提交为 `7102fdd`；本轮已提交 `f9fb158`（正式更新目标过滤）、`a019c36`（更新助手故障验收）、`6ab19ea`（三核心验收入口）、`f6d76a7`（mihomo 原生配置检查边界）、`64d35b7`（CI 测试路径/SDDL 规范化）、`4fd63bb`（原生运行错误进入主界面状态）和 `dc0a1d2`（简化托盘）。以下证据不代表所有交接项已经通过。

用户随后要求删除托盘中的三核心操作，仅保留打开 Ladder、打开程序目录和退出。`dc0a1d20fbc27b07ed08c443ba8369846927b577` 已完成该实现及独立 review，本地双架构重建、包校验和真实菜单操作通过；最终 `0b35edd` 上的三核心运行后托盘退出与恢复清理也已通过。下述 `4fd63bb` 的构建及校验保留为对应生产版本的历史证据。

真实编辑检查另发现默认关联为 Unknown/OpenWith 时绕过 Notepad 回退。`0b35edde8a97193c0277b1d42ff3be6d2013b41e` 已仅修复 `desktop.rs`：优先查找有效 edit/open 关联，忽略 Unknown/OpenWith 回退，无有效处理程序时直接打开系统记事本，有效用户关联仍交给 Shell。独立 review、3 项 desktop 纯测试及 fmt/clippy 通过。该提交已只推送 GitHub `dev`，本地双架构增量构建及独立包校验通过，[push CI](https://github.com/senzyo/ladder/actions/runs/44000070418)也成功：fmt/clippy 和默认测试 **125 通过、0 失败、4 忽略，23.52 秒**，日志为 `target/handoff-20261117/actions-push-0b35edd.log`。同一 AMD64 EXE 的真实编辑器复验已通过；未触发新的 manual 或标签发布，不因该单文件改动重复原三核心全量沙盒矩阵。

新 Release 的真实菜单已验证：仅有上述三项；右键菜单打开 Ladder、关闭窗口隐藏后左键恢复、打开正确程序目录 `C:\ladder-real` 均通过。证据为 `output/simple-tray-menu.png`、`output/simple-tray-directory.png`；最终三核心运行后的托盘退出与 DNS 恢复/清理结果见后文。真实检查更新显示“已是最新版本”，截图为 `output/check-update-current-and-dns-on.png`；截图中开关状态本身不证明实际 DNS 接管。

`0b35edd` 真实编辑器复验：未关联的 `.yaml`、路径包含中文和空格的 `.json` 均直接打开系统记事本，没有 OpenWith 对话框。另单独验证文件监视：先打开编辑器，再回 Ladder 完成绿色验证，随后在记事本换行保存，回 Ladder 后旧验证标签清除。证据为 `output/editor-fallback-yaml.png`、`output/editor-fallback-json-unicode.png`、`output/editor-valid-before-save.png`、`output/editor-validation-cleared-after-save.png`。实际浅/深色主题、125% 缩放和默认最小窗口未见裁切，尝试缩小时仍保持约 900×600 逻辑尺寸；证据为 `output/product-settings-light.png`、`output/product-settings-dark.png`。该结果不外推到未测的其他缩放比例。

真实下载官方 mihomo `1.19.33` 后，从 `1.19.34` 切换到 `1.19.33`，旧绿色验证结果清除；command 61 的状态与日志确认切换成功，随后已切回 `1.19.34`。证据为 `output/version-validation-before.png`、`output/version-validation-after.png`、`output/version-switched-settings.json`。前缀连续保存也实测落盘：Tab 补全默认前缀后追加 `qa` 再保存，command 59 确认最终值为 `https://gh-proxy.org/qa/`；清空再保存后 command 60 确认 `github_proxy` 为空，随后使用直连下载。`qa` 路径只用于保存核对，没有向该路径发出网络请求。输入 `v` 时的格式错误提示也实际出现；证据为 `output/prefix-last-saved.png`、`output/prefix-second-settings.json`、`output/prefix-cleared-settings.json`。

最初未找到 `gh`；用户随后完成配置，GitHub CLI `2.104.0` 登录和仓库写入权限已核实。本轮只向 GitHub 推送 `dev`，未修改 `main`、推送标签或创建 Release；本地按用户要求只编译 Release 包，`-Publish` 不属于本轮任务，不再作为阻塞条件。

新 `Build and release` 工作流已实际运行：

- `6ab19ea8daca2a1c80bad635cee25da9584164fc` 的 [push 检查](https://github.com/senzyo/ladder/actions/runs/43991029651)中 fmt/clippy 通过，测试为 118 通过、2 失败、4 忽略。失败分别是错误消息中的路径别名和测试账户 SID 的 SDDL 表示差异，日志为 `target/handoff-20261117/actions-push-6ab19ea.log`。
- `64d35b7ec95357dfb078585359876afbb4ef89c0` 仅修正这两项测试后，[远端复测](https://github.com/senzyo/ladder/actions/runs/43993016349)成功，fmt/clippy 和默认测试 120 通过、0 失败、4 忽略，日志为 `actions-push-64d35b7.log`。同提交的[手动双架构工作流](https://github.com/senzyo/ladder/actions/runs/43993826825)通过 `--ref dev` 成功触发；因生产修复随后提交，主动取消这次过时构建以放行最终提交检查，不记为构建失败。
- `4fd63bb98157245073d06c641a976f4d247eb211` 已只推送到 GitHub `dev`，其 [push 检查](https://github.com/senzyo/ladder/actions/runs/43994241082)成功：fmt/clippy 通过，默认测试 **124 通过、0 失败、4 忽略**。日志为 `actions-push-4fd63bb.log`。随后核实远端分支仍指向该提交，再触发[最终手动双架构打包](https://github.com/senzyo/ladder/actions/runs/43995383477)，API 确认实际运行 SHA 与上述完整提交一致。该运行已成功：完整检查再次 124 通过、0 失败、4 忽略，AMD64/ARM64 均打包成功，AMD64 的生产更新助手 6 场景全部通过。日志为 `actions-manual-4fd63bb.log`；Release 作业按手动触发规则跳过，没有创建标签或 Release。
- `dc0a1d20fbc27b07ed08c443ba8369846927b577` 已显式只推送到 GitHub `dev`，新 [push 检查](https://github.com/senzyo/ladder/actions/runs/43998391327)成功：fmt/clippy 通过，默认测试 **124 通过、0 失败、4 忽略，24.22 秒**，日志为 `actions-push-dc0a1d2.log`。本次不再重复触发 manual，待真实界面及 DNS 验收后按确认的标签方案执行最终构建/预发布；目前仍未推送标签。

未改变默认分支或合并 main；不能用旧版 [CI 记录](https://github.com/senzyo/ladder/actions/runs/42049042931)替代本轮证据。

### 本轮已执行的本地与沙盒检查

- `f9fb158` 修复了标签为 alpha/beta/rc、GitHub 却标记 `prerelease=false` 时会被接受为正式更新的问题。新增回归先在旧实现复现错误，修复后 5 项软件更新纯逻辑/临时文件测试通过；保留旧 `2.34.0` 过滤及 `3.0.0-alpha.1` 到正式版的比较检查。
- `a019c36` 的 `scripts/test-release.ps1` 使用唯一命名的无害模拟程序，原样执行生产更新助手。Windows PowerShell 5.1 下成功更新、重启失败回滚、无提交标记取消、校验失败、文件占用、助手 ready 后取消再正常退出，共 6 场景通过；逐场景确认设置、三核心配置及 mihomo 引用资源保留。真实 Ladder 仅被读取版本和打包，没有由该脚本启动。证据：`target/handoff-20261117/update-helper.log`。这不是完整 Ladder 联网升级，也不是 Rust 持有者 Drop 的端到端验收。
- 补齐 mihomo 进程用例后，沙盒默认套件首次为 119 通过、1 失败、2 忽略；失败定位为测试等待 PID 文件的竞态。修复后同套件为 **120 通过、0 失败、2 忽略**；加入真实联网入口后，默认并行套件又以 **120 通过、0 失败、4 忽略** 完成。证据：`output/full-tests-initial.log`、`output/full-tests.log`、`output/full-tests-parallel.log` 及对应结果 JSON，均位于 `target/handoff-20261117/`。忽略项不算通过，后续生产代码修改仍须在最终提交重新执行套件。
- `64d35b7` 在 DNS 损坏记录测试中加入目录别名以稳定复现路径断言失败，再允许错误消息标识原路径或规范化后的同一文件；保留实际恢复与损坏内容断言。ACL 测试用 Windows 原生 SDDL 转换规范测试账户 SID，避免原始 RID 500 与 `LA` 别名被当成不同权限；生产 ACL 脚本、掩码和精确比较规则未改。修复前两个最小复现均失败，修复后 DNS 16 项、原 ACL 用例及格式检查通过；实际 runner 结果以远端复测为准。
- `4fd63bb` 的运行错误呈现修复经独立 review；AMD64 沙盒完整默认套件为 **124 通过、0 失败、4 忽略**，证据为 `output/native-errors-full.log`。新增模拟回归覆盖进程仍运行时的原生错误、等级识别、旧日志边界和错误保留；该结果不代替真实生产界面复测。
- `dc0a1d2` 简化托盘后，fmt/clippy/no-run 与独立 review 通过；沙盒 command 32 完整默认套件为 **124 通过、0 失败、4 忽略，31.18 秒**，证据为 `output/simple-tray-full.log`、`output/simple-tray-full-result.json` 和 `output/done-32.json`。
- 本轮最新 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings` 和测试 `--no-run` 编译通过。早期双架构构建记录 `target/handoff-20261117/release-build.log` 属于中间工作区产物；生产源码提交 `4fd63bb` 后已重新执行 `scripts/release.ps1`，两种架构 Release 构建和打包均成功，证据为 `native-errors-release-build.log`。随后只读校验 ZIP 白名单、产品版本、PE 架构及 SHA-256，结果见下表和 `local-packages-4fd63bb/verified-packages.json`；核验没有启动 EXE。包内 README 为构建时工作区版本，后续 README 变化应重新打包并更新校验值。
- 生产版 Ladder `3.0.0-alpha.1` 已在 AMD64 沙盒真实启动，默认进入 mihomo；真实版本列表获取及通过界面下载 mihomo `1.19.34` 已操作验收。启动证据为 `output/product-start.json`、`output/product-start.log`。该结果不包含完整托盘、DNS/TUN、联网自升级或全部界面组合验收。用户明确没有 ARM64 机器，ARM64 仅有构建证据，实机启动保留为未验收。

本地 `4fd63bb` 源码构建的两个 ZIP 均仅包含 `LICENSE`、`README.md`、`ladder.exe`，EXE 产品版本均为 `3.0.0-alpha.1`，对应 `.sha256` 内容与实算一致：

| 文件 | 字节 | PE 架构 | SHA-256 |
| --- | ---: | --- | --- |
| `ladder-amd64-v3.0.0-alpha.1.zip` | 6537112 | `0x8664` | `3aad5193a9be53caac6186558820447e2bc2167138480f641430b82bf7a71fd0` |
| `ladder-arm64-v3.0.0-alpha.1.zip` | 6157008 | `0xaa64` | `2b074382026ba5ac264c16421b311b16e8f1da87ba5eb36cfbeaab6449e28a81` |

同一生产提交的远端手动工作流四个资产已实际下载，只读独立核对均通过：文件集合恰为两 ZIP 及对应 `.sha256`，ZIP 白名单、EXE 产品版本和 PE 架构与上表规则一致。证据为 `target/handoff-20261117/actions-manual-4fd63bb/verified-packages.json`。远端 runner 与本地构建的 ZIP 不要求逐字节相同，分别保留以下校验值：

| 远端文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `ladder-amd64-v3.0.0-alpha.1.zip` | 6514093 | `af8d1588da0392e89ed4500f2e5c415df0503e2497f6a7ee700af86a705220a7` |
| `ladder-arm64-v3.0.0-alpha.1.zip` | 6157396 | `6c2b80ff5f850f1f0a30b504cbae6de5f1188c19535152ffdc1936065c7e550b` |

`dc0a1d2` 的本地双架构构建日志为 `target/handoff-20261117/simple-tray-release-build.log`。编译前记录 `src/lib.rs`、`src/runtime.rs`、`src/ui.rs`、README 的 SHA-256，构建后核对未改变且与该提交一致。两包白名单、产品版本 `3.0.0-alpha.1`、PE 架构及对应 `.sha256` 均独立核对通过；证据为 `local-packages-dc0a1d2/verified-packages.json`。AMD64 EXE 已用于沙盒真实菜单验收，文件 SHA-256 为 `de34c1792f4799602031fa81fc927158cd83489abd3c16e1ae13358df97f14a0`。

| `dc0a1d2` 本地文件（历史） | 字节 | PE 架构 | SHA-256 |
| --- | ---: | --- | --- |
| `ladder-amd64-v3.0.0-alpha.1.zip` | 6521048 | `0x8664` | `86bdfd4371afb03025b37ad6ee8d7eaf073e3bc71f20846fae46f96250c63587` |
| `ladder-arm64-v3.0.0-alpha.1.zip` | 6140208 | `0xaa64` | `5bc692b1cad6527c1115bf881e1dc3a9180beae57e567a02ada097320d6b8e9a` |

最新生产提交 `0b35edd` 两架构增量构建成功，日志为 `target/handoff-20261117/editor-release-build.log`；含 `desktop.rs` 在内的 5 个源码/README 文件在构建前后哈希一致，且与该提交一致。两包白名单、产品版本、PE 和 SHA-256 独立核对通过，清单为 `local-packages-0b35edd/verified-packages.json`。沙盒使用的 AMD64 EXE 哈希为 `66b28b4b56ed6b4914e612d1b10d953915e6d6d8857e3f49946ec2040dc5daa7`。

| 最新本地文件（`0b35edd`） | 字节 | PE 架构 | SHA-256 |
| --- | ---: | --- | --- |
| `ladder-amd64-v3.0.0-alpha.1.zip` | 6522609 | `0x8664` | `cc0b1e6f25d3b050860feee4eb1916b1df301ff96bf694a1c6f048fe64211092` |
| `ladder-arm64-v3.0.0-alpha.1.zip` | 6140691 | `0xaa64` | `2040a85c4371de25e873b0875a0378ab24d55093a145feac2e048f008c90a871` |

### 真实核心与加速首轮结果

首轮共 9 项，AMD64 沙盒执行，使用真实官方核心。汇总为 `target/handoff-20261117/output/official-results.json`，逐项证据为同目录 `*-direct.log`、`*-prefix.log`。

| 核心与通道 | 版本 | 地址 | 结果与范围 |
| --- | --- | --- | --- |
| mihomo 稳定版 | `1.19.34` | 直连 | 失败：JSON、YAML 合法配置通过，语法错误被拒绝；缺失本地 provider 的配置仍返回 `Ok(())`，触发验收断言，后续生命周期步骤未在此轮完成。 |
| mihomo 预览版 | `alpha-3c0d10a` | 直连 | 失败：同样接受缺失本地 provider 的配置；不能记录为配置验证通过。 |
| mihomo 稳定版 | `1.19.34` | 加速前缀 | 失败：版本列表完成，下载阶段报告请求受限；未完成该地址的安装验收。 |
| sing-box 稳定版 | `1.14.4` | 直连 | 通过：下载安装、版本、合法/语法错误/缺失资源配置、启动/重启/切换配置/停止及自动启动。 |
| sing-box 预览版 | `1.15.0-alpha.13` | 直连 | 通过：同上。 |
| sing-box 稳定版 | `1.14.4` | 加速前缀 | 通过：真实版本列表、安装包下载、隔离安装及版本探测；此项不包含生命周期测试。 |
| Xray 稳定版 | `26.11.9` | 直连 | 通过：下载安装、版本、合法/语法错误/缺失资源配置、启动/重启/切换配置/停止及自动启动。 |
| Xray 预览版 | `26.9.30` | 直连 | 通过：同上。 |
| Xray 稳定版 | 未取得 | 加速前缀 | 失败：版本列表阶段报告请求受限，未开始下载。 |

以上首轮日志中的四项直连生命周期均为 `0 peer listeners checked`，其本身不能证明多核心共存。首轮 mihomo 两项失败是原测试要求 `-t` 拒绝缺失 provider，但实测其原生命令接受了该配置；用户已选择保持原生检查行为，不增加 Ladder 本地文件预检。`f6d76a7` 明确记录这条边界，缺失资源改由真实运行错误呈现验收。

随后对受限前缀各做一次受控重试：mihomo `1.19.34` 的版本列表、下载、隔离安装和版本探测通过；Xray 仍在版本列表阶段报告 API 限流，保留失败并停止连续重试。证据为 `output/prefix-install-retry-once-results.json`、`output/mihomo-stable-prefix-retry-once.log`、`output/xray-stable-prefix-retry-once.log`。结合首轮 sing-box 成功证据，当前为 mihomo/sing-box 加速链路通过、Xray 未通过。

### 三核心共存复测与 mihomo 运行错误

在上述选择下重新执行真实三核心套件，稳定版及预览版两组均通过。证据为 `output/coexist-stable.log`、`output/coexist-preview.log` 及对应 `*-result.json`。版本仍为稳定组 mihomo `1.19.34`、sing-box `1.14.4`、Xray `26.11.9`，预览组 mihomo `alpha-3c0d10a`、sing-box `1.15.0-alpha.13`、Xray `26.9.30`。

两组均逐核验证启动、重启、切换配置、停止与自动启动，并在操作目标核心时检查另外两个真实监听端口，日志明确为 `2 peer listeners checked`。因此已有真实多核心共存证据；DNS/TUN 在这些用例中关闭，恢复功能仍未由该结果验收。

新增“进程仍运行时错误进入状态”的模拟用例先失败后通过，证据为 `output/live-error-red.log`、`output/live-error-green.log`；修复提交为 `4fd63bb`。随后真实生产界面复测通过：三核心缺失资源的原生错误均进入主界面和对应日志，mihomo 进程仍运行，Xray/sing-box 立即退出后错误仍保留。截图为 `output/mihomo-missing-after-main.png`、`output/mihomo-missing-after-log.png`，以及同目录的 `xray-missing-after-main.png`、`xray-missing-after-log.png`、`sing-box-missing-after-main.png`、`sing-box-missing-after-log.png`。sing-box/Xray 的批量验证均在缺资源失败后继续检查合法配置，通过证据为 `output/sing-box-batch-validation.png`、`output/xray-batch-validation.png`。

command 30 以真实 mihomo 和回环 HTTP 服务对照 provider 存在/缺失两种情况；两者原生 `-t` 均成功且请求后进程存活。规则均为 `RULE-SET,local,REJECT` 后接 `MATCH,DIRECT`：provider 存在时 REJECT 命中、目标服务器收到 0 次请求；provider 缺失时记录加载错误，MATCH/DIRECT 命中、目标服务器实际收到 1 次请求并返回 200。该用例已直接验证缺资源后的 fallback，不只是源码推断。证据为 `output/mihomo-fallback/20261117-013649-4d8f2b/results.json` 及两组配置/检查/运行日志；流量只发往沙盒回环服务。

另在沙盒对真实 Xray `26.11.9` 和 sing-box `1.14.4` 执行 18 项原生命令资源探针，全部符合预期。Xray 缺失 `geoip.dat` 或 `geosite.dat` 时，检查与运行均退出 23；sing-box 缺失 JSON/SRS 规则或使用已移除的 GeoIP/Geosite 配置时，检查与运行均退出 1；资源齐全的基线检查成功并能持续运行。证据为 `output/run-18.log` 和 `output/resource-evidence/`。这些命令按核心原生行为验证，没有加入 Ladder 本地文件预检，也不单独证明 Ladder 主界面/日志集成通过。

“完成”指该项所有列明验收完成；“部分”指已有明确子项证据；“未执行”指尚无该项要求的实际运行证据。

旧三核心托盘重启已在真实 AMD64 沙盒执行，旧 PID、TUN/GUID 和运行副本清理，同伴与源配置 SHA 保持不变；证据为 `output/run-27.log`、`output/run-28.log`、`output/run-29.log` 和 `output/tun-stages/`。由于用户撤销了这些托盘操作，此处仅作为旧行为历史，不再列入新版本剩余验收要求。

command 31 的旧托盘配置切换仅完成 Snapshot 采集，不能将采集 PASS 写为全部清理通过：mihomo 新进程出现、旧运行副本消失且同伴不变，旧 TUN `79e3d5` 当时仍以 down 状态被枚举。随后 command 37 的只读 PnP 查询及 command 39 的接口表中均不再有该旧接口或设备，说明该条目后来自行消失，未取得清理策略缺陷证据。

command 33 的恢复断言是假阴性，原因是验收脚本在 PowerShell 5.1 中没有展开 JSON 数组；两种地址族的 static 快照本就与基线相同，DNS 实际未改变。修正 helper 后 command 36/39 的 Restored(v3) 检查通过；这时仅打开开关而没有核心运行，不将其计为实际接管后恢复，也不记录为产品 DNS 恢复失败。

随后 command 40–44 完成第一轮真实 mihomo DNS 接管与正常停机验证：启动后两个地址族均指向回环；关闭开关但不停止核心时继续保持回环；停止 mihomo 后恢复到基线，源配置 SHA 保持不变。证据为 `output/run-40.log` 至 `output/run-44.log` 和对应 `output/tun-stages/` 快照。

command 45–49 验证 sing-box 运行并实际接管 DNS，强制结束核心后 Stopped 与 Restored 检查通过。command 50–54 验证 Xray 接管后强制结束 Ladder：Job 核心树退出，DNS 当时仍保持回环且恢复记录保留；重新打开 Ladder 后 Restored 检查通过。证据为各自 `output/run-45.log` 至 `output/run-54.log` 和对应快照。这里分别覆盖正常停止、核心异常退出、应用异常退出后重启恢复，未将强杀应用误记为即时恢复。

最终生产程序 `0b35edd` 的 command 62–65 逐核启动及 Loopback 检查通过：三个核心、TUN 与运行副本共存，后启动的核心没有重建同伴，两个地址族的 static/effective DNS 都处于回环，原始恢复记录、源配置 SHA 和默认路由保持不变。随后通过 UI 关闭 DNS 开关、关闭窗口隐藏，再从简化托盘退出。command 66 的原始 Restored/PnP 结果已通过，但汇总脚本未指定 UTF-8 读取中文 JSON，导致该次命令失败；保留失败记录，仅修正取证脚本编码后用 command 67 重采，不是产品修复，也没有重新启动三核心。

最终完整证据 `output/final-exit/final-exit-20261117-022643-090/summary.json` 在 02:26:53 UTC 记录 `Errors=[]`，`NoOwnedProcesses`、`NoRuntimeCopies`、`NoDnsJournals`、`NoTunInterfaces` 全为 true，Ladder/三个核心进程均为空；DNS 回到原 DHCP/effective 基线，源配置校验及默认路由不变。独立复核同目录 `pnp-20261117-022653-164.json`，MIB/PnP/NetAdapter 均无 TUN，读取错误为 0，配置选择保留。截图为 `output/final-three-core-running.png`、`output/final-tray-menu.png`。此结果以 command 67 为准；后续独立配置验证 QA 的启动不属于该生产目录退出状态。

最后在独立 `C:\ladder-validation-qa` 用同一生产 UI 和原样编译的无害 fixture 验证超时/取消，生产与 fixture 源码均未改变。fixture 的验证分支等待 90 秒：command 69 记录 PID 2220 以 `-t` 运行，command 70 已无该进程，距离创建 77.627 秒，早于自然结束；主界面实际显示“配置验证超时，请检查配置引用的资源是否可用”，截图为 `output/validation-timeout.png`。该证据结合生产 30 秒 deadline 验证超时处理，稀疏快照不用于声称实测恰为 30.000 秒，也不声称 app.log 记录了这条 UI 内存日志。

首次 command 71/72 在验证开始 36.021 秒后才退出，只证明超时后的正常退出，不计主动取消通过。command 73–75 重新验证：PID 1200 于 02:33:19.995 UTC 创建，command 74 确认验证仍在执行；Ladder 于 02:33:40.810 退出，相隔 20.815 秒；command 75 在 02:33:46.592（相隔 26.597 秒）记录 `Processes=[]`，`ApplicationRunning=false`、`FixtureRunning=false`。两者均早于 30 秒 deadline，排除超时或 90 秒自然结束，独立复核确认验证中托盘退出会提前结束应用及子进程，无遗留。快照为 `output/validation-ui/20261117-023326-597/snapshot.json`、`output/validation-ui/20261117-023345-795/snapshot.json`。这是生产 UI 截止与退出链验收，不是官方核心解析超时的专项验收。

完整 Ladder 联网升级当前无法完成：运行版本为 `3.0.0-alpha.1`，官方 latest 为旧实现 `2.34.0`，没有可接受的更高正式版本。本轮不发布一个正式版来制造验收条件；生产助手的无害程序测试与更新目标纯逻辑测试不能代替前后两个真实 Ladder 版本的联网升级。

| 项目 | 状态 | 已有证据与剩余工作 |
| --- | --- | --- |
| 1. GitHub Actions 与本地发行包 | 部分 | 最新 `0b35edd` 本地双架构构建、独立包校验及 push CI 125/0/4 通过；前序远端双架构及助手场景保留对应提交证据。本地 `-Publish` 不在本轮范围；标签发布未执行。 |
| 2. 三核心真实运行及 mihomo 补齐 | 部分 | 真实共存/生命周期、三种 DNS 正常/异常恢复，以及最终三核同时运行后退出、DNS 恢复、进程/TUN/副本/记录清理通过。更新失败后的实际恢复等未单独执行场景保留边界。 |
| 3. 简化托盘 | 完成 | 三项菜单、打开/隐藏后恢复、正确程序目录和三核运行后的退出/清理全部实测通过；旧托盘证据只作历史。 |
| 4. 软件完整升级 | 部分 | 过滤回归和无害程序生产助手 6 场景通过；当前没有可接受的更高正式版本，完整联网升级无法完成，Rust Drop/退出失败完整链路等未验收。 |
| 5. GitHub 加速实测 | 部分 | mihomo/sing-box 前缀列表、下载和安装通过；实际格式错误提示、连续保存最后值与清空直连落盘通过。Xray 重试仍被 API 限流，保留失败且不连续重试。 |
| 6. 真实配置验证 | 完成（所列场景） | 真实核心配置/资源/错误、批量继续、编辑及版本失效通过；生产 UI 搭配无害 fixture 的超时与验证中退出取消通过，明确不等于官方核心卡死专项验收。 |
| 7. 发行包与真实界面 | 部分 | 最新 `0b35edd` 本地双架构 Release 与包校验通过；AMD64 编辑器、浅/深色、125% 最小窗口及前述功能已有实际证据。ARM64 无机器，实机启动未验收。 |
| 8. 全仓死代码审查 | 完成 | 全仓静态审查已完成；唯一特性删除候选经编译证伪并恢复，无可确认删除项，恢复路径保留。 |
| 9. 文档与最终交付 | 待提交/标签 | 本轮代码和既定验收已收尾，交接改为已完成项与未验证边界；三份最终文档待 review/提交，随后执行确认的标签预发布并补记结果。 |

既定生产修改与本轮验收已结束；当前仅等待最终文档提交和确认后的标签预发布。完整联网升级、ARM64 实机启动、Xray 加速及明确未执行的其他场景保留受限/未验证结论，不笼统宣称所有场景完成。
