# 依赖升级与 Rust 规范重构：实施交接

更新日期：2026-10-25。本文保存执行顺序、证据和当前进度，供同一会话压缩上下文后或新会话接手时使用。用户最新指令优先；代码与实际验证结果优先于陈旧状态记录。

## 当前状态与执行范围

- 用户选择 B：先按建议顺序分批升级依赖，验证通过后按已定稿方案重构。
- 用户已于 2026-10-23 指示按照暂存区文档开始执行，并自行补装构建工具。S0、D1–D4、R1–R4 已通过；2026-10-25 最新续做完成 R4 内部 API 对齐，Clippy、77 项测试及双架构 release 构建通过。本轮在 R4 验证边界收尾，下一步 R5；R5–R7 尚未实施。
- 用户于 2026-10-24 明确提醒旧版 `ladder.exe` 正在运行，并允许分会话执行；2026-10-25 授权先提交 Git，再按最新版交接继续，最新指令为“按最新版交接文档继续”。本轮按 R4 边界推进。未启动新编译的应用；后续也不得为验证而启动它、终止现有实例或核心、修改真实 DNS/注册表/计划任务。实际托盘与 DNS 验证留到可恢复的隔离环境。
- **始终读取工作区最新版本。** 依赖与首批整理已按用户指令提交为 `531f3a2`（`chore: 升级依赖并完成首批 Rust 风格整理`）；R2/R3 已提交为 `d8ea64b`（`refactor: 整理 Rust 导入并拆分应用职责模块`）。本轮 R4 以该干净提交为起点，源码和进度文档均留在未暂存区，未自动提交。typed COM 迁移已由用户明确暂缓，保留原 `hr != 0` 判定和手写接口，不新增迁移所需 features。
- 工作目录：`C:\Users\admin\Documents\GitRepos\ladder`；分支 `main`，本轮 HEAD 为 `d8ea64b`，父提交为 `531f3a2`。接手时用 `git log -1` 和 `git status` 确认实际状态，不假设分支或文件始终不变。
- 最初五份规划/术语文档及前轮实现已包含在 `531f3a2` 中，R2/R3 源码与文档在 `d8ea64b` 中。R4 的未提交修改是最新进度，换 checkout/worktree 时必须一并传递。
- 不自动 commit、push、发布，不更改全局工具链或系统配置。分批是独立审查和验证的单位，不表示自动创建提交。
- Windows 10 22H2、PowerShell 7；已观察到 rustc 1.99.0。执行前读取适用的 AGENTS.md；PowerShell 输出中文前设置 UTF-8。

## 接手顺序

1. 阅读本文并检查 `git status`、当前依赖版本与工作区差异，保留用户的既有修改。
2. 阅读 [重构方案](google-rust-style-refactor.md)、[规范逐条清单](google-rust-style-checklist.md)、[领域术语](../../GLOSSARY.md)、[故障处理 ADR](../adr/0001-poison-and-emergency-cleanup.md)。不重新选择已经确认的架构和故障策略。
3. 从下面第一个未完成批次继续。先验证、记录结果，再进入下一批；失败时先定位并处理本批回归，不能写成已通过。
4. 每批结束更新本文的进度表及证据记录。换会话或上下文压缩后先重读这些文件，不依赖上一轮的口头总结。

当前直接入口是下文“新会话续做入口”。不要重复升级依赖或重做已完成的 R2–R4，也不要将职责迁移和闭包访问等同于故障策略和 Windows 边界已完成。

## 执行批次

| 批次 | 范围 | 完成条件 | 状态 |
| --- | --- | --- | --- |
| S0 | 建立未升级基线 | 记录工具链、锁文件、现有差异；检查格式、Clippy、测试、amd64/arm64 release 构建及字节数；记录已有失败或缺失环境 | 已完成，58 个测试通过 |
| D1 | TLS 安全补丁 | 精确更新下面三项传递依赖；保持 ureq 3.3.0；确认依赖差异、检查/测试/双架构构建及相关公告匹配 | 已完成，58 个测试及双架构构建通过 |
| D2 | sha2 0.11.1 | 验证既有哈希向量与跨缓冲读取、检查双架构构建；不撤销现有分块哈希写法 | 已完成，4 个哈希测试及双架构构建通过 |
| D3 | ureq 3.5.0 与超时适配 | 保持正常慢下载可完成、响应头等待和读取停滞有界；用本地模拟服务验证，不调用真实核心更新 | 已完成，64 个测试及双架构构建通过 |
| D4 | zip 9.0.5 与 API 适配 | 验证白名单、恶意路径、虚报解压大小、必需文件、备份保留、替换失败回滚；完成依赖阶段验收 | 已完成，66 个测试及双架构构建通过 |
| R1 | 重构基线 | 复用依赖维护后的验证和产物体积 | 已完成，复用 D4 |
| R2 | 格式与机械命名 | 100 列格式、名称、导入及 expect 上下文分批审查 | 已完成，Clippy、66 项测试通过 |
| R3 | 职责迁移 | 按模块映射迁移代码和测试，先保留原锁与错误语义 | 已完成，66 项测试及双架构构建通过 |
| R4 | 内部 API 对齐 | 具名结果、闭包访问、must_use、derive、隔离线程测试等 | 已完成，77 项测试及双架构构建通过 |
| R5 | 故障策略 | 中毒处理、无 tracing 紧急路径及隔离测试 | 未开始 |
| R6 | Windows 边界 | 收缩 unsafe 与补安全契约；typed COM 已明确延期 | 未开始，COM 部分延期 |
| R7 | 最终验收 | 逐项复核规范、不变量、自动检查、双架构构建及环境允许的实际验证 | 未开始 |

测试应针对本批风险，复用已通过的检查；只在后续修改、失败或未解决风险需要时重复。构建使用独立 `CARGO_TARGET_DIR`，防止 build.rs 覆盖现有运行目录中的配置。不能把缺少 ARM64 原生运行环境写成原生验证已通过。

保留两份基线：S0 的原始基线与 D4 完成后的依赖基线。后续 COM/重构的体积变化相对依赖基线计算，避免把依赖升级引起的变化混入重构结果。

## 依赖目标与适配要点

下列是 2026-10-23 已核实的发布版本，不是无限追随 latest 的授权。正式更新前核对撤回/新安全公告；若需要超出这些范围，说明具体原因。

| 包 | 原锁定版本 | 目标版本 | 要点 |
| --- | --- | --- | --- |
| rustls | 0.23.41 | 0.23.46 | TLS 协议处理修复；现有 ureq 范围允许补丁升级 |
| rustls-webpki | 0.103.13 | 0.103.16 | 拒绝已失信中间 CA 公钥 |
| webpki-roots | 1.0.8 | 1.0.11 | 移除已失信根证书路径；修复下限为 1.0.10 |
| sha2 | 0.11.0 | 0.11.1 | ARM64 后端修复；MSRV 为 1.88，当前 zip 已要求该版本 |
| ureq | 3.3.0 | 3.5.0 | 更新源码配置与注释，不能只刷新 Cargo.lock |
| zip | 8.6.0 | 9.0.5 | 需要修改主版本要求及处理 ZipFile::name 的新返回类型 |

### D1：精确更新安全依赖

本次只读评估使用 OSV 按锁文件 112 个 registry 包的精确版本查询，匹配 4 个包、9 条 RustSec 公告；官方 registry 撤回检查为 112 项成功、无 yanked。它不是完整利用性审计，也不是升级后的验证报告。

TLS 三项实际依赖链为 `ladder → ureq → rustls / webpki-roots`，`rustls → rustls-webpki`。ureq 3.3.0 的约束允许上述补丁，无需先升级 ureq。避免无目标的全量 `cargo update` 意外带入 ureq 的新超时语义；逐包精确更新后检查全部锁文件差异，包括解析器必须调整的传递依赖。

- [rustls 加密层边界公告](https://rustsec.org/advisories/RUSTSEC-2026-0285.html)：这不是认证绕过；0.23.46 也包含后续补丁。
- [rustls-webpki 证书公告](https://rustsec.org/advisories/RUSTSEC-2026-0382.html)。
- [webpki-roots 信任根公告](https://rustsec.org/advisories/RUSTSEC-2026-0381.html)。
- zip 的三个公告涉及整体 extract API 的符号链接/Unix 权限；本项目 Windows 手工白名单提取未调用这些 API，不能把版本匹配直接写成项目可利用。[0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html)、[0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html)、[0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html)。

### D2：sha2

0.11.1 修复 AArch64 后端常量加载不符合 Stacked Borrows 的问题。项目发布 arm64，因此补丁有实际相关性；现有 `Digest::update/finalize` 写法保持。[发布版 changelog](https://raw.githubusercontent.com/RustCrypto/hashes/sha2-v0.11.1/sha2/CHANGELOG.md)。

### D3：ureq

`src/update.rs` 当前通过旧阶段 deadline 行为实现空闲超时。新版本中 `timeout_recv_body(30s)` 是整个响应体的总预算，`timeout_send_request` 也不再替响应头兜底；原样升级会改变正常慢下载行为。

按新版公开 API 使用响应头阶段超时与 `timeout_per_read`，保持响应体总预算为 `None`，保留现有 30 秒参数及小响应总超时。核查连接/TLS/重定向阶段，删除旧行为解释。测试覆盖响应头卡死、响应体中途停滞、持续传输超过设定测试总时长和重定向；允许测试注入较短时限，验证语义而不让每个用例等 30 秒。不要顺带重新设计 DNS 解析或重试策略。[3.5.0 changelog](https://raw.githubusercontent.com/algesten/ureq/3.5.0/CHANGELOG.md)、[版本文档](https://docs.rs/ureq/3.5.0/ureq/)。

### D4：zip

`ZipFile::name()` 由 `&str` 变为 `ZipResult<Cow<'_, str>>`，现有告警格式化需要显式处理；不要为记录非法名称而新增 panic。9.0.1 加强 by_index 读取大小约束，9.0.4 加强 Windows enclosed_name 路径校验。项目仍取末级文件名并执行白名单，不改成整体 extract；不顺带改变核心文件清单、Xray 自带 geo 文件更新、备份与回滚语义。新版读取大小检查不等于应用级最大解压体积限制。[大小检查](https://github.com/zip-rs/zip2/pull/1022)、[路径检查](https://github.com/zip-rs/zip2/pull/1066)。

### 暂不升级的项

- windows 0.62.2、windows-sys 0.61.2 已是核查时最新稳定版。虽有相应 COM 接口，但成功判定不能保持原契约；该迁移已明确延期，详见 R6 预审记录。
- windows-registry 0.6.1 暂不迁移到 0.100.0。serde、serde_json、winresource 更新也不自动纳入 B；它们是另行安排的维护项。
- tracing、tracing-subscriber、tempfile 在核查时已为最新稳定版。

## 重构时不能遗漏的约束

完整约束以重构方案为准，以下只是接手时优先注意的要点：

- 单 crate、保留全局状态、平铺叶模块；术语 Core/CoreSettings/CoreStatus/ConfigSelection；格式与改名分批，行宽 100。
- 正常行为保持。独立缺陷另列；批准的项目故障行为例外只有按资源处理中毒锁，以及普通 panic 复用无 tracing 的紧急清理。依赖自身的安全修复及必要适配按 D1–D4 单独记录。
- 闭包访问器使用 FnOnce；设置修改与保存仍在同一次可变访问内串行进行，不能旧快照锁外覆盖；保存失败不新增回滚，原有失败通知锁范围先保持。
- 紧急清理不读应用状态、不写 tracing、不弹 UI；下游进程和 DNS 调用链同样必须无日志；保留 release abort。测试使用独立子进程/局部锁和清理替身，不修改真实 DNS、注册表或终止用户核心。
- typed COM 迁移已延期；本轮保留手写接口、属性布局与现有释放规则，不引入隐藏 vtable API 绕过决定。若未来恢复迁移，仍须保持 VT_LPWSTR、Vec 借用及严格 HRESULT 判定，并重新讨论阻碍；原方案的 ManuallyDrop/RAII 约束仅在恢复迁移时适用。
- Windows 上游 VARIANT/PROPVARIANT 的 Drop 改为 Free/Owned 的变更虽已合并，尚未进入当前已发布 windows 0.62.2；不要将仓库总 release 的 changelog 当作该 crate 已发布证明，也不改用 Git 依赖。
- amd64 原生 COM/通知验证与双架构构建是 typed COM 切换门槛；ARM64 原生运行另列。真实网络/托盘/DNS操作只能在可恢复的隔离环境验证，不能以日常用户环境冒充测试环境。

## 每批证据记录

### 2026-10-23：S0 首次执行，等待补齐构建环境

- 分支 `main`，HEAD `f04809ca29c6605959e30e3a65e55a3d35bf29db`；开始时仅五份文档已暂存，未暂存区无差异。未执行 commit、push 或改变暂存区。
- `rustc 1.99.0 (b940084d7 2026-09-28)`，`cargo 1.99.0 (5f94df478 2026-08-27)`；主机为 `x86_64-pc-windows-msvc`，已安装 x86_64 和 aarch64 两个 MSVC target。未更改工具链或全局环境。
- 原始 `Cargo.lock` SHA256：`13314BFE61676296D35BCCB8FC6296B05159DBB8E0D1DD4E49C48F19B6D65161`。六项原锁定版本与上表一致，锁文件未修改。
- 静态确认现有 58 个测试（更新 24、配置 10、进程 11、状态 11、日志 2）；测试不执行应用 `main`，未发现真实 DNS、注册表、计划任务改写或核心终止调用。测试未能完成编译，因此通过数量仍为未验证。

| 检查 | 独立 CARGO_TARGET_DIR（相对仓库） | 结果 |
| --- | --- | --- |
| `cargo fmt --check` | 不适用 | 退出 0，通过 |
| `cargo clippy --all-targets --locked -- -D warnings` | `target/codex-validation` | 退出 1：`linker link.exe not found` |
| `cargo test --locked` | `target/codex-validation` | 退出 1：同上，未运行测试 |
| `cargo build --release --locked` | `target/codex-validation-release` | 退出 1：同上，无本轮产物 |
| `cargo build --release --locked --target aarch64-pc-windows-msvc` | `target/codex-validation-arm64` | 退出 1：同上，无本轮产物 |

环境核查及历史证据：

- `vswhere` 仅列出 VS Build Tools 2026（18.10.12224.181），已选组件为 MSBuild/.NET；x64 C++ 和 ARM64 C++ 组件查询为空。标准安装路径下没有 VC 工具目录，也没有 Windows Kits/10 的 SDK 文件；沙箱外只读复核这些明确路径得到相同结果。
- 旧构建输出记录了 VS 2022 的 `VC/Tools/MSVC/14.44.35207` 和 Windows SDK `10.0.26100.0/x64/rc.exe`。用户随后确认，为运行新 C#/.NET 项目安装了 VS 2026，并删除了 VS 2022 相关安装。这解释了历史编译成功与当前缺失工具的差异。
- 原 `target/release/ladder.exe` 为 2,130,944 字节，原 ARM64 产物为 1,866,752 字节，两者时间为 2026-08-26；仅记录历史参考，不能用作本轮 S0 成功基线或后续体积比较基准。
- 当前有 LLVM 的 clang-cl/lld-link，但缺少 MSVC CRT 和 Windows SDK，不能据此宣称可完成 MSVC 构建。未运行会删除 dist 的 `ci.ps1`，未启动应用或修改真实网络环境。

依赖只读预检已完成：六个目标版本在官方 crates.io 精确版本 API 中均存在且 `yanked = false`，对应 RustSec 公告和版本 API 与方案一致；未发现必须扩大目标版本范围的证据。此项不代替升级后的检查，也不是对整个依赖树重新完成的安全审计。证据：[rustls](https://crates.io/api/v1/crates/rustls/0.23.46)、[rustls-webpki](https://crates.io/api/v1/crates/rustls-webpki/0.103.16)、[webpki-roots](https://crates.io/api/v1/crates/webpki-roots/1.0.11)、[sha2](https://crates.io/api/v1/crates/sha2/0.11.1)、[ureq](https://crates.io/api/v1/crates/ureq/3.5.0)、[zip](https://crates.io/api/v1/crates/zip/9.0.5)。

当时的下一步是等待用户补齐工具链；此阻塞现已解除，见下一节。代理未安装全局组件。

### 2026-10-24：S0 通过

- 用户自行安装后，VS Build Tools 为 18.11.1，MSVC 为 14.51.36231；SDK 安装日志于 2026-10-23 23:59:31 记录退出 0，SDK 目录版本为 10.0.26100.0。初次复查期间 SDK 尚在安装，出现过 `kernel32.lib` 缺失；安装结束后该问题消失。
- 使用 `vcvarsall.bat x64` 和 `vcvarsall.bat x64_arm64` 为各构建子进程加载环境，不修改全局 PATH。Rust/Cargo 与原始锁文件哈希保持上述值。
- 在上一节三个独立目标目录运行同样的 Clippy、测试及 release 命令，全部退出 0；`cargo fmt --check` 和 `git diff --check` 通过。58 个测试全部通过，0 失败、0 忽略，运行 2.83 秒；未执行实际应用。
- S0 原始 release 基线：amd64 **2,129,920 字节**，ARM64 **1,873,920 字节**。两个产物分别位于 `target/codex-validation-release/release/ladder.exe` 和 `target/codex-validation-arm64/aarch64-pc-windows-msvc/release/ladder.exe`。
- ARM64 仅完成交叉编译链接，未验证原生运行。下一步按 D1 精确更新三项 TLS 依赖；ureq 保持 3.3.0。

### 2026-10-24：D1 通过

- 仅 `Cargo.lock` 中三包变化：rustls 0.23.41 → 0.23.46、rustls-webpki 0.103.13 → 0.103.16、webpki-roots 1.0.8 → 1.0.11；ureq 仍为 3.3.0，无其他版本变化。
- 分别运行 `cargo update -p <包> --precise <目标版本>`，然后 `cargo fetch --locked`。普通沙箱进程报 Schannel `SEC_E_NO_CREDENTIALS`；经审批在沙箱外执行同样更新成功，未禁用 TLS 校验或修改全局网络配置。
- 更新版本均覆盖前述公告修复下限。原 rustls 0.23.41 另外匹配 [0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html)、[0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html)、[0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html)，均在 0.23.46 修复；[0347](https://rustsec.org/advisories/RUSTSEC-2026-0347.html) 仅影响 0.23.44–45，不能算作旧版本已匹配。公告匹配不等于本项目可利用。
- 沿用 S0 的独立目录与 vcvars 环境，Clippy、全部 58 个测试、两架构 release 构建均退出 0，`git diff --check` 通过；源码未改动，复用 S0 格式检查。
- D1 release 大小：amd64 **2,133,504 字节**，ARM64 **1,877,504 字节**。下一步 D2，仅更新 sha2 补丁。

### 2026-10-24：D2 通过

- 执行 `cargo update -p sha2 --precise 0.11.1` 和 `cargo fetch --locked`，均退出 0；相对 D1 仅 sha2 0.11.0 → 0.11.1，分块哈希源码未修改。
- Clippy、`cargo test --locked update::tests::test_sha256_file`（空文件、abc、跨 64 KiB 缓冲区的大文件、文件不存在共 4 项）及双架构 release 构建均退出 0；复用未改动区域在 D1 通过的测试。
- D2 release 大小：amd64 **2,133,504 字节**，ARM64 **1,877,504 字节**，均与 D1 相同。ARM64 后端只验证编译链接，未声明原生运行通过。

### 2026-10-24：R6 预审决策更新

- 本机 windows 0.62.2 的公开 typed COM 方法将非负 HRESULT 转为成功，不能保留原实现 `hr != 0` 失败的严格判定。读取生成的 vtable 又需要 `#[doc(hidden)]` 项，与清单冲突。
- 用户明确选择：**保持现有错误判定，暂缓这段 COM 接口迁移**。因此该接口段保留手写实现，不为迁移新增 features；其他模块与安全边界重构照常进行。此项为用户确认的延期，不计作 typed COM 验证通过。

### 2026-10-24：D3 通过

- `Cargo.toml` 的 ureq 最低兼容版本改为 3.5，以匹配新超时 API；精确更新锁定 ureq 3.3.0 → 3.5.0，其发布依赖要求带来 base64 0.22.1 → 0.23.1、ureq-proto 0.6.0 → 0.6.5，无其他包变化。
- `src/update.rs` 分别限制建连、发送和接收响应头，以 `timeout_per_read` 处理停滞，正文总预算明确为 `None`。保留 30 秒参数、小响应全局预算、分块哈希、生产代理与重试策略；删除旧 deadline 传导说明。
- 新增 6 项本地 HTTP 测试：响应头卡住、持续慢头、正文停滞、持续慢正文、重定向到慢正文、重定向后头卡住。服务使用 127.0.0.1 动态端口和独立结束时限，测试只在该请求关闭代理，不改变进程环境或调用真实核心更新。
- `cargo fmt --check`、Clippy、全部 **64 个测试**、双架构 release 构建、`git diff --check` 均通过；本地 HTTP 6 项先单独通过，再纳入全测。
- D3 release 大小：amd64 **2,146,816 字节**，ARM64 **1,896,448 字节**。下一步 D4，完成 ZIP API 适配与读取大小/路径回归。

### 2026-10-24：D4 通过，建立依赖维护后的基线

- `Cargo.toml` 保留原 deflate/features 结构，zip 主版本改为 9；精确锁定 zip 8.6.0 → 9.0.5。新增 nix 0.31.3 与其构建依赖 cfg_aliases 0.2.2，仅由 zip 的 macOS/BSD 目标依赖引入；两个 Windows 目标的 `cargo tree -i nix` 均无条目。
- `src/update.rs` 显式处理 `name()` 的 Result，告警失败使用固定占位文字；纠正 `enclosed_name` 注释，不新增绝对路径拒绝策略或应用级解压体积上限。
- 路径穿越测试扩为 6 种普通/反斜杠/verbatim/UNC 样本；新增 Stored/Deflated 虚报解压大小测试，验证错误后当前文件及 `.old` 不变、所有 `.new` 清理；新增正常 Deflated 对照。既有白名单、必需文件、可选文件与替换失败回滚测试继续通过。
- 首次全测暴露 D3 测试服务的 Windows 非阻塞 socket 继承竞争：偶发 `WouldBlock`。修正为 accept 后显式 `set_nonblocking(false)`，保留测试超时参数和独立服务时限，并补充错误上下文；不更改生产代码。修正后全套 **66 个测试连续 3 轮通过**，Clippy、格式和差异检查通过。
- 两架构 release 构建均退出 0；后续竞争修正仅涉及 `#[cfg(test)]` 区域，复用其构建结果。依赖基线大小：amd64 **2,153,984 字节**，ARM64 **1,900,032 字节**。后续重构体积比较以此为准。
- 依赖基线 Cargo.lock SHA256：`ADA9DA1D7911EFE76D72B1B420C31A6587B55F9499FB1BE7AFF4B53390FD117B`。所有指定版本到位，未升级 windows、windows-registry、serde 等排除项。R1 复用本节基线，进入 R2 格式与机械命名。

### 2026-10-24：R2 部分完成，按用户要求收尾交接

- R2A：`rustfmt.toml` 的 `max_width` 从 120 改为 100，edition 仍为 2024；单独执行 `cargo fmt`，对 `src` 和 `build.rs` 做格式化，不混入命名改动。
- R2B：`ConfigAction` → `ConfigSelection`、`ProcessState` → `CoreStatus`、设置类型 `Core` → `CoreSettings`、`ComGuard::new` → `try_new`、`spawn_bg` → `spawn_background`；同步相关内部变量/函数名。核心身份类型 `Core` 保持不变，未改变配置键或序列化格式。
- 命名完成后 `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、`git diff --check` 全部退出 0；**66 个测试通过，0 失败、0 忽略，2.89 秒**。锁文件哈希与 D4 相同。
- 独立只读复核对照格式快照确认：R2B 仅 main/settings/state/tray 四文件发生预期改名与必要换行，配置字段和序列化键、状态映射、线程名称、锁范围及执行顺序均保持；其余六个源码文件一致，未发现混入语义变化。
- R2 尚余导入整理、必要的 expect 不变量说明等机械检查。模块拆分、闭包状态 API、中毒策略、unsafe 边界均未实施；现有 `state.rs`、返回锁守卫和中毒恢复等旧实现仍然存在，不能视为遗漏已完成批次的回归。
- R2 未重建双架构 release，当前两个验证目录中的 release 产物仍代表 D4；最后完整发布构建证据见 D4，不宣称最新命名后的 release 已构建或运行。
- 为分开审查而保存了忽略目录下的源文件快照：`target/codex-batches-20261024/d4/`（格式前）、`r2-format/`（格式后、命名前）、`r2-names/`（本次收尾）。可用 `git diff --no-index <前目录>/src <后目录>/src` 比较；退出码 1 只表示有差异。快照非 Git 提交，换 checkout 时不会自动带走；缺失时以工作区和上述记录为准，不重放整批替换脚本。
- 所有实现与进度修改保持未暂存，原暂存区仍只有最初五份文档；未提交、推送、运行应用或触碰日常网络配置。

## 新会话续做入口

### 从 R5 故障策略开始

1. 读取工作区的方案/进度文件、术语和 ADR，检查相对 `d8ea64b` 的未提交差异；最新验证见下文 R4 记录。环境和代码未变时无需重新建立 S0 或重做 R2–R4。
2. 主要入口已是 `app.rs`、`commands.rs`、`logging.rs` 和 `app_state.rs` 等平铺叶模块；`main.rs` 只做装配，`state.rs` 已删除、`error.rs` 已更名为 `app_error.rs`。不得按旧文件布局重新运行提取脚本。
3. R4 已引入具名结果和 `FnOnce` 闭包访问，`APP` 已私有，状态初始化经 `app_state::initialize`。从 `app_state::with_state`、logging 的 write/flush、settings 的警告缓存及 emergency/process/dns 清理链进入 R5；按资源替换中毒恢复，并接入无 tracing 的紧急清理。
4. 每个批次验证后更新本文；可以再次在完整批次边界交接，无须在单一会话耗尽上下文。

R4 所有模块已完成汇合，最新源码在当前未暂存区，无需从子代理或临时脚本恢复内容。没有必须等待的后台构建/测试。切换 checkout 时应使用 `d8ea64b` 加本轮 R4 未提交修改。

### 后续实现注意点

- R3 已迁移：`StatusIcons` 在 tray，继续显式退出释放；不能改成依赖静态变量 Drop。`ensure_exists` 已成为 process 私有辅助；进程查询与核心状态查询也在 process，`state.rs` 不再保留。
- R4 已保留的约束：设置更新和 `save()` 在同一次可变闭包内，保留既有错误日志/通知与保存失败后的内存变化。核心模式切换仅在 `Some(Ok(()))` 时记成功。规则集下载后只修改当前仍存在的条目；忙守卫一直覆盖后台任务结束。R5 不得改变这些锁范围和错误契约。
- R4 具名结果已完成：`ReleaseInfo`、`DownloadAsset`、`ParsedSettings`、`MenuSelection` 等。Xray 搜索保持“首次匹配发行版缺少 tag 即报错”的顺序，借用资产直到确定结果再复制；保留 digest 和版本号的容错，不改为严格解析。
- R5：全链路核查 process 与 DNS 的无日志底层函数，普通包装仍保留原日志。枚举失败、注册表打开/写入失败等返回信息需足以保留普通路径原有诊断。紧急路径仍按 sing-box、xray、恢复 DHCP、刷新 DNS 顺序，不访问 AppState 或 Child。
- R5 测试：局部 Mutex 用工作线程 panic 制造中毒；日志 write/flush 固定错误且无递归 tracing，另一输出层仍收到事件；启动警告队列返回固定降级消息。退出和 panic 钩子使用有父进程超时的独立测试子进程，并注入只记录步骤的清理替身。不得调用真实清理。
- R5 待替换的旧实现：app_state、logging 的 write/flush、settings 警告队列仍有四处 `into_inner()`；emergency 钩子与下游清理仍有 tracing。R4 已删除公开 APP、返回锁守卫的接口以及测试的 catch_unwind/临时全局 hook，不能据此把中毒/紧急清理策略写成已完成。
- R6：typed COM 明确延期，其余 FFI 安全边界仍需处理；方案末尾的图标、消息循环、XML、TUN 等独立缺陷不得夹带修复。
- R7：逐条更新规范清单，保留未验证项；以 D4 的 2,153,984 / 1,900,032 字节为重构体积基准。ARM64 原生运行与真实托盘/DNS 验证当前均未做。

### 已验证可用的本机构建方式

每次在构建子进程中初始化 VS 环境，不能假设普通 PowerShell 已能找到 link.exe；不改全局 PATH。

```powershell
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
cargo fmt --check
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target/codex-validation'
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked'
```

需要发布构建时，amd64 将 `CARGO_TARGET_DIR` 设为 `target/codex-validation-release`，使用 `vcvarsall.bat x64` 和 `cargo build --release --locked`；ARM64 设为 `target/codex-validation-arm64`，使用 `vcvarsall.bat x64_arm64` 和 `cargo build --release --locked --target aarch64-pc-windows-msvc`。只编译，不执行产物。不使用会删除 dist 且含交互流程的 `ci.ps1`。

依赖已下载齐全。若后续必须联网且再次遇到 Schannel `SEC_E_NO_CREDENTIALS`，按实际工具审批流程处理，不禁用 TLS 校验。曾仅在更新子进程中设置 `CARGO_HTTP_MULTIPLEXING=false` 避免 HTTP/2 临时错误，不是项目或全局配置要求。

### 建议使用的技能

- `pwsh-tips`：本项目为 Windows/PowerShell，按其引号、UTF-8、临时脚本约定执行命令。
- `handoff`：需要再次换会话时整理增量状态；用户已指定现有交接文档，优先更新本文，不另造互相矛盾的进度副本。
- 只有代码或已有材料不能回答具体依赖 API 问题时，再使用官方文档检索技能；当前版本目标与 COM 延期均已决定，无须重新做全量依赖研究。

## 2026-10-25 续做证据

### 用户要求的提交

- 接手时上轮 18 个改动文件全部已暂存，未暂存区无差异；检查 `git diff --cached --check`、文件范围与锁文件哈希后，执行 `git commit -m 'chore: 升级依赖并完成首批 Rust 风格整理'`，退出 0，提交 `531f3a2`。
- 该提交包含依赖升级、100 列格式、主要名称及五份文档；没有 push。随后的 R2 余项和 R3 均留在未暂存区，未扩大“先提交”授权为自动逐批提交。
- R2/R3 验证并交接后，用户再次明确要求“先提交git”；因此把后续源码和文档一并提交。此前各批记录中的“未暂存/未提交”描述的是当时状态，当前以本节和实际 Git 状态为准。

### R2 导入与 expect 上下文完成

- 移除 logging 的 tracing prelude 通配导入；SubscriberExt/SubscriberInitExt、Digest、Read、格式化 Write、BufRead、Windows 扩展 trait 等按实际使用范围导入。仅内联测试保留 `use super::*`。
- 类型改为显式导入；普通跨模块函数保留模块限定，Win32 名称明确的函数保留直接导入例外，`io::Error` 等有歧义的类型保持限定。
- 构建脚本 OUT_DIR 与测试准备/断言结果的裸 unwrap/unwrap_err 改为有上下文的 expect/expect_err；fallback 家族、中毒策略、测试条件及生产控制流不变。设置字段/serde 键未变。
- `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、`git diff --check` 全部退出 0；**66 项测试通过，0 失败、0 忽略，2.97 秒**。
- R2 完整源文件快照：`target/codex-batches-20261025/r2-complete/`，用于独立审查之后的纯迁移；未建立新的 Git 提交。规范中的 API、文档、unsafe 及数值字面量等最终逐条复核仍由 R4–R7 收口。

### R3 职责迁移完成

- `main.rs` 只声明 18 个叶模块并调用 `app::run()`；原启动/单实例装配移到 app，命令分发与后台任务移到 commands，日志与原两项测试移到 logging，COM 与单实例守卫各自拆出，原 panic 钩子移到 emergency。
- 原 state 拆成 app_state（全局状态/标志及 2 项测试）、core（核心身份/清单/状态及 2 项测试）、config_selection（选择/扫描及 3 项测试）、windows_strings；StatusIcons 移入 tray，进程查询/状态计算及 4 项测试移入 process，`ensure_exists` 改为 process 私有函数。原 state 已删除，error 已更名 app_error。
- 调用路径与必要的 `pub(crate)` 可见性同步调整，托盘回调指向 commands；Cargo.toml 仅更新 panic 钩子的注释路径，依赖和 release 配置不变。正常顺序、锁范围、忙守卫生命周期、旧中毒/panic 行为与手写 COM 均保留。
- 独立只读审查对照 R2 快照：66 个测试名称一一对应、无新增/丢失/重复，测试体词法 token 相同；238 个函数在预期模块路径与入口改名归一化后，签名和函数体一致；98 项类型/常量/静态/别名声明在忽略迁移必要可见性后相同。人工复核启动/退出、忙守卫与 COM 生命周期、设置保存及规则集锁范围，未发现迁移遗漏或 R4/R5 混入。该项是静态迁移证据，不代替 Windows 实际运行验证。
- `cargo fmt --check`、Clippy、全部 **66 项测试**（0 失败、0 忽略，2.87 秒）、`git diff --check` 通过。测试按职责分布为 app_state 2、config_selection 3、core 2、logging 2、process 15、settings 10、update 32；没有新增或丢弃原覆盖目的。
- 使用既定两个独立目标目录与 vcvars 环境构建，amd64 和 ARM64 release 均退出 0；不执行产物。amd64 **2,153,984 字节**、ARM64 **1,900,032 字节**，相对 D4 两者均 **0 字节 / 0%** 变化。
- Cargo.lock SHA256 仍为 `ADA9DA1D7911EFE76D72B1B420C31A6587B55F9499FB1BE7AFF4B53390FD117B`。R3 源文件快照为 `target/codex-batches-20261025/r3-complete/`；这两个快照位于忽略目录，不随 Git 自动传递。
- Windows 实际托盘/模式切换/配置选择/退出/DNS、ARM64 原生运行仍未验证；typed COM 已延期，未宣称其切换门槛通过。当时下一步为 R4，已由下节续做完成。

### R4 内部 API 对齐完成

- 接手时 `main` 分支 HEAD 为 `d8ea64b`，工作区干净；本轮修改 12 个源码文件及三份进度/方案文档，未暂存、提交或推送。Cargo.toml、Cargo.lock、实际 settings.json 和运行目录保持未改动。
- app_state 将 `APP` 设为私有，新增 `initialize`、`with_app_state`、`with_mut_app_state`。两个访问器接收 `FnOnce`，状态借用和锁守卫不能逃出闭包；私有 `with_state` 同时用于局部状态测试。app、commands、process、tray 的全部调用点已迁移；未初始化时仍走原通知/静默/空句柄分支。中毒恢复暂保留，明确由 R5 单独变更。
- 核心模式修改、规则集时间修改与各自 `save()` 仍在同一可变闭包中，失败日志/通知仍在原锁范围，保存失败不回滚内存。核心模式以 `Option<Result<..>>` 将失败传到外层，只有 `Some(Ok(()))` 记录成功；规则集仅更新当前仍存在的条目。进程终止、下载、状态查询和菜单显示继续在锁外，忙守卫覆盖完整后台任务。
- update 新增私有 `ReleaseInfo { version, assets }` 和 `DownloadAsset { url, expected_hash }`；借用 Xray 候选资产，选定后才复制，保持首次匹配缺 tag 即失败、预发布版选择、digest 和版本容错。settings 新增 `ParsedSettings { settings, warnings }`，Download 改 derive Default；配置格式、默认值和警告顺序不变。
- tray 的菜单选择/快照/命令组合具名化，禁用参数使用 `MenuItemState`；命令 ID、菜单顺序、图标、flags 与释放时机保持。为资源守卫和必要查询补 `must_use`，提取复杂循环头的迭代变量并补契约。展开释放测试改用工作线程和 join，不再替换全局 panic 钩子或调用 catch_unwind。
- 新增 11 项测试：app_state 4 项验证未初始化不执行闭包、拥有值移入/取出与释放锁、串行保存不覆盖前一修改、保存失败传出且保留内存；settings 1 项验证多字段修正与警告顺序，并强化默认设置一致性；update 6 项验证版本/资产映射、缺失字段、首匹配选择及 digest 容错。均使用局部状态、临时目录或固定 JSON，不运行应用入口或真实清理。
- `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、`git diff --check` 全部退出 0；**77 项测试通过，0 失败、0 忽略，2.88 秒**。测试分布为 app_state 6、config_selection 3、core 2、logging 2、process 15、settings 11、update 38，原 66 项覆盖保留。
- 两个既定独立目标目录内的 amd64/ARM64 release 构建均退出 0。产物大小：amd64 **2,153,472 字节**，相对 D4 **-512 字节 / -0.0238%**；ARM64 **1,899,520 字节**，相对 D4 **-512 字节 / -0.0269%**。构建后只修正一条注释中对零长度缓冲区 panic 的过强描述，未改变可执行代码，复用本轮构建与测试结果。
- 独立只读审查确认状态所有权、保存与通知锁范围、失败传播、忙守卫和规则集回写不变量；另行复核 release/资产映射、Xray 首匹配、digest 容错与菜单行为，未发现运行行为回归。零长度缓冲区的文档问题已修正为调用前提，未新增断言或修复独立缺陷。
- Cargo.lock SHA256 仍为 `ADA9DA1D7911EFE76D72B1B420C31A6587B55F9499FB1BE7AFF4B53390FD117B`。R4 源文件快照为 `target/codex-batches-20261025/r4-complete/`，位于忽略目录，不随 Git 自动传递。
- 本轮在 R4 验证边界收尾。R5 中毒/无日志紧急清理、R6 unsafe 边界、R7 逐项验收尚未开始；typed COM 保持延期。Windows 实际托盘/模式切换/配置选择/退出/DNS 和 ARM64 原生运行仍未验证；未启动新应用、终止旧实例或核心、修改真实 DNS/注册表/计划任务。

后续每批追加：批次状态、修改文件、实际版本差异、执行命令与退出结果、测试数量及关键用例、两架构产物字节数（适用时）、未验证项、遇到的回归及修正、下一步。

无法验证的门槛保持未完成，说明缺少什么；只有新的实质冲突或必需信息缺失才重新询问用户，不为已确认批次重复索要同意。
