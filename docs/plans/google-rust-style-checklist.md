# Google Rust Style Guide 逐条对照

来源：[规范原文](https://google.github.io/styleguide/rustguide.html)，首次读取日期：2026-10-23，最终复核读取日期：2026-10-25。执行范围以[重构方案](google-rust-style-refactor.md)和用户确认的决策为准；依赖维护、批次及验证证据见[实施交接](dependency-upgrade-and-refactor-handoff.md)。

本表记录 2026-10-25 的逐条静态复核结果。**已落实**表示原“调整”项已有对应代码；**已满足**表示复核后维持现状；**条件适用**表示依据条款自身的适用条件或例外保留；**不适用**表示专属代码库要求或项目不存在相应接口；**延期**表示用户已明确暂缓，不能计为完成。静态复核不等于所有既有缺陷已修复。

R1–R6 及 R7 静态复核已落实到工作区。本轮 `cargo fmt --check`、Clippy（all-targets、locked、拒绝 warnings）、全部 **84 项测试**（0 失败、0 忽略，2.92 秒）及 amd64/ARM64 release 构建均通过；具体命令和产物证据见实施交接。typed COM 迁移继续延期；Windows 隔离实测、ARM64 原生运行及已知独立缺陷仍单列，不据静态注释或交叉构建宣称通过。

## 适用原则与代码组织

| 规则 | 最终静态状态 | 当前代码证据或适用理由 |
| --- | --- | --- |
| 规范适用范围与目标 | 已满足 | 单个 Windows 桌面应用采用通用规则；重构范围和行为例外已记入方案及 ADR 0001，不套用其他代码库的构建体系 |
| 遵守公开 Rust 风格 | 已落实 | `rustfmt.toml` 固定 edition 2024、100 列；R2 整理导入和命名，后续 API、文档、unsafe 对应下表；本轮格式和 Clippy 检查通过 |
| 按应用/库上下文应用 API 规则 | 条件适用 | `Cargo.toml` 和 `main.rs` 为单 binary；内部设置、命令、FFI 资源类型没有跨团队库消费者，保留直接接口，不额外引入通用抽象 |
| 优先小 crate | 不适用 | Chromium 专属；按用户决定保留单 Cargo crate，没有拆出仅服务此应用的微型 crate |
| rust_library 平铺文件布局 | 不适用 | 没有 Chromium `rust_library` 目标；`main.rs` 声明平铺叶模块是本项目已确认方案，不作为遵循其构建布局的证明 |
| 叶模块按主要自定义类型命名 | 已落实 | `app_error`、`app_state`、`com_guard`、`single_instance_guard`、`config_selection` 按主要类型命名；`dns`、`logging`、`process` 等无单一主要类型，按职责命名 |
| 非叶模块不实现大量 API | 已落实 | `main.rs` 只声明模块并调用 `app::run()`；启动装配在 `app`，命令分发在 `commands`，日志、COM、单实例和紧急清理分别进入叶模块；`emergency::test_support` 仅为故障测试辅助模块 |
| 避免 glob import | 已落实 | 删除 tracing prelude 通配导入，使用 `SubscriberExt`、`SubscriberInitExt` 等具体 trait；仅内联 `#[cfg(test)] mod tests` 保留允许的 `use super::*` |
| 一条 use 只导入同一模块中的项 | 已满足 | 如 `std::sync::{Mutex, OnceLock}`、`crate::app_state::{self, FlagGuard}` 只含同层项；不同层级分别导入，未引入嵌套 use 树 |
| trait 尽量在局部导入 | 已落实 | `update::sha256_file` 内导入 Digest/Read，`to_hex` 内导入格式化 Write；`windows_strings::wide`、`process::hidden_command` 内导入 Windows 扩展 trait；多处实现共用的 `logging::Write` 和测试共用 Write 保留模块范围 |
| 跨模块顶层函数带模块限定 | 已落实 | 调用使用 `app_state::with_app_state`、`process::stop_all`、`toast::show_toast`、`windows_strings::wide` 等模块路径；`CreatePopupMenu`、`CoInitializeEx` 等名字自带明确来源的 Win32 API 保留原条款允许的直接导入 |
| 类型导入后不带模块限定 | 已落实 | `Core`、`CoreStatus`、`ConfigSelection`、`AppError`、`PathBuf` 等显式导入；`io::Error`、`io::ErrorKind`、`serde_json::Error` 等需区分来源的名称保留限定，Windows Error 需要时别名为 `WindowsError` |
| 使用合适的导入机制 | 不适用 | Chromium 专属导入宏与构建机制不适用于 Cargo 项目，保持普通 Rust `use` |

## 格式、所有权与函数接口

| 规则 | 最终静态状态 | 当前代码证据或适用理由 |
| --- | --- | --- |
| 使用对应格式化工具 | 已落实 | 保持 stable rustfmt、100 列及 edition 2024；十六进制数值字面量改为小写，日志 ANSI 使用 `\x1b` 转义。无自定义 `macro_rules!` matcher；规范中的不稳定格式选项按其 stable 例外人工复核，不开启 nightly。HRESULT 日志的 `:08X` 是输出格式契约，未改变 |
| 退出自动格式化的条件 | 已满足 | 源码没有 `rustfmt::skip`，未以个人排版或 Windows 绑定为由跳过自有代码的格式化 |
| 避免不必要的 Rc/Arc | 条件适用 | `CappedLogWriter` 的 `Arc<Mutex<CappedFile>>` 服务于 tracing 多线程 writer 共享；故障测试的 Arc 连接工作线程与检查方。其余应用状态继续用已确认的 `OnceLock<Mutex<_>>`，不新增共享所有权层 |
| 借用优于深拷贝 | 已落实 | `select_xray_release` 先借用候选资产、选中后才复制；释放状态锁后执行菜单、下载和后台操作所需的 `PathBuf`、设置及配置快照仍保留，避免借用逃出锁或跨线程悬空 |
| 不可变数据使用灵活引用 | 已满足 | 文件操作使用 `&Path`，文案/地址使用 `&str`，资产查询使用 `&[Value]`；未改为不必要的 `&String`、`&Vec<_>` 或强制拥有型参数 |
| 返回迭代器或拥有的集合 | 条件适用 | `find_json_configs` 返回排序去重后的 `Vec<PathBuf>`，警告/PID 枚举返回拥有的 Vec；这些结果需要排序、跨锁或跨调用使用，保留集合而不暴露临时借用迭代器 |
| 参数采用迭代器或切片 | 条件适用 | `find_json_configs` 接收目录切片，`stop_processes` 接收 `&[Core]`，release 选择接收 `&[Value]`；已满足已知调用方需要，不为假设中的库调用者扩大泛型接口 |
| 使用 Into 参数的条件 | 已满足 | `hidden_command(program: impl AsRef<OsStr>)` 同时服务程序名及路径调用；其他函数保留具体引用类型，没有为了减少局部转换而普遍引入 `Into<String>` 等参数 |
| enum 优于含糊的 bool 参数 | 已落实 | 托盘 `append_item_or_disabled` 使用 `MenuItemState::{Enabled, Disabled}`；`core_status(installed, running)` 的两个明确业务条件和 Win32 固定布尔签名保持，不机械包装所有 bool |
| 选择 Option、Result 或 panic 报错 | 已落实 | 未初始化状态仍返回 None，常规 IO/更新错误继续 `Result<_, AppError>`；状态中毒释放守卫后清理退出，日志中毒返回固定 IO 错误，警告缓存中毒返回固定提示；panic 钩子共用无日志清理后调用原诊断钩子 |
| newtype 与类型别名 | 已满足 | `commands::ExeDirFn` 为可读的函数签名别名，`FlagGuard`、`CappedLogWriter` 包装各自真实生命周期/写入职责；路径和文案继续使用标准类型，没有无用途的字符串 newtype |
| 具名字段优于多字段元组 | 已落实 | 更新结果为 `ReleaseInfo`、`DownloadAsset`，解析结果为 `ParsedSettings`，菜单使用 `MenuSnapshot`、`MenuSelection`、`CoreMenuItems`；局部 `enumerate`/`zip`、匹配状态和短小测试样本元组按清晰度例外保留 |

## trait、并发与宏

| 规则 | 最终静态状态 | 当前代码证据或适用理由 |
| --- | --- | --- |
| 按需要增加 trait 实现 | 已满足 | 设置模型保留 serde 与快照所需 Clone/Debug/Default；Core 的 Copy/比较用于身份分支；未批量给资源守卫添加 Clone/Default 等会破坏契约的实现 |
| 标准 trait 优于同义自定义方法 | 已满足 | `AppError` 实现 `Display`、`Error` 和标准 From；日志 writer 实现 `Write`，设置采用 `Default`，没有另造等价的错误、写入或默认值 trait |
| 能 derive 则优先 derive | 已落实 | `Download` 已改为 derive Default，`Settings`、`CoreSettings`、`CoreDownload` 同样 derive；`Log`、`Retry`、`Ruleset` 需要非零/特定默认值，保留调用共享默认函数的手写实现 |
| 泛型与 dyn Trait 的选择 | 条件适用 | 状态访问用 FnOnce，枚举/清理替身用具体泛型闭包；标准错误/钩子和 tracing 所需类型保持原接口，无为动态分发而引入的应用框架，也无需要反向泛型化的业务接口 |
| trait 与固有 impl 的边界 | 已满足 | Core 查询、Settings 加载保存、AppState 子进程操作放在固有 impl；只在标准 trait 或依赖扩展点中实现 trait，没有为按文件分组而创建 trait |
| 扩展 trait 的使用 | 已满足 | 无自定义外部类型扩展 trait；Windows 字符串/进程等功能使用标准库既有扩展 trait，未给外部类型添加混淆来源的伪固有方法 |
| Copy/Clone/Default/比较等重要 trait | 条件适用 | 资源守卫保持不可复制，`ComGuard` 以私有标记约束在初始化线程释放；Core/CoreMode 的比较和复制有实际调用。`StatusIcons` 保留现有句柄快照 Copy，但删除方法明确唯一释放的 unsafe 契约；不把含 String/IO 错误的 AppError 强制做成 Copy |
| 主要 From 转换配显式构造器 | 条件适用 | `AppError::Io`、`AppError::Json` 已是显式具名构造入口，From 仅为 `?` 转换服务；无需再建立同义工厂方法，亦未引入仅能通过 From 构造的新领域类型 |
| 选项集合实现 Default | 已满足 | Settings 及其选项集合已有默认值，serde 缺字段策略复用这些默认函数；`DownloadAsset` 的必填 URL 不虚构无意义的空默认结果 |
| 禁止继续使用中毒锁内部值 | 已落实 | `app_state::try_with_state` 显式 drop PoisonError 后返回无数据标记；日志 write/flush 不再操作中毒文件，警告缓存不再重放旧队列；源码没有 `into_inner()` 或 `clear_poison()` 调用 |
| 不使用 async/await | 不适用 | Chromium 专属；当前实现仍为同步 IO 与后台线程，没有新增异步运行时 |
| 不通过标准库启动线程 | 不适用 | Chromium 专属；`commands::spawn_background` 和规则集/测试线程继续使用标准库，保留原后台任务与忙守卫生命周期 |
| 使用 fuchsia_sync::Mutex | 不适用 | Fuchsia 专属；本项目继续使用标准库 Mutex 并落实明确中毒策略，不添加 Fuchsia 依赖 |
| 宏的使用 | 已满足 | 使用 tracing、format/write、serde derive、matches/assert 等成熟宏；没有自定义 `macro_rules!` 或过程宏框架，因而没有需要调整的 matcher 定义 |

## 命名、文档与其他语言特性

| 规则 | 最终静态状态 | 当前代码证据或适用理由 |
| --- | --- | --- |
| 方法命名 | 已落实 | 领域类型统一为 Core/CoreSettings/CoreStatus/ConfigSelection；核心信息用 `label`、`exe_path`、`active_config`、`artifacts` 等查询名。HTTP GET、进程枚举和本地版本执行等真实动作不机械改成字段 getter |
| 生命周期命名 | 已满足 | 日志 trait 的 `for<'a> LookupSpan<'a>` / `FormatFields<'a>` 表达短局部约束；`'static` 用于固定元数据、原子守卫及线程闭包。无承载复杂多方关系却使用含糊名称的公开生命周期参数 |
| 泛型参数命名 | 已满足 | 状态访问 `F, R` 紧邻 FnOnce 和返回值约束，日志 `S, N` 紧邻 Subscriber/FormatFields，缓冲区长度用 const `N`；没有为重命名字母而改变清晰的惯用局部约束 |
| 扩展 trait 命名 | 不适用 | 没有自定义扩展 trait；使用的 `CommandExt`、`OsStrExt`、`SubscriberExt` 等来自标准库或依赖，保留其名称 |
| 初始化函数命名 | 已落实 | COM 构造器为 `ComGuard::try_new() -> Result<_, AppError>`；`SingleInstanceGuard::acquire`、状态 `initialize`、`acquire_busy` 按初始化/获取动作区分，不统一改成 `new` |
| 避免缩写 | 已落实 | 后台调度改为 `spawn_background`，主要领域名按 GLOSSARY 对齐；DNS、COM、PID、URL、Win32 类型/字段及 FFI ABI 名称保持通用含义，不与绑定刻意分叉 |
| 文档编写时机、形式和契约 | 已落实 | 各叶模块有职责说明；状态访问说明 FnOnce、不可重入与保存失败语义；紧急清理说明 release abort/尽力清理，Windows 入口说明线程、句柄和缓冲区前提。修正 CoreArtifacts 的 Xray geo 文件注释，使其符合当前实际覆盖行为 |
| unsafe 的必要性、封装与说明 | 已落实，保留风险 | 托盘、进程、DNS、日志、COM/单实例和通知将 unsafe 缩到 FFI、原始指针或相邻调用，并说明 ABI、长度、所有权、线程和释放依据；需要调用方保证的入口保留 unsafe 与 `# Safety`。GetMessageW 的 -1 路径明确未完成安全证明，既有资源缺陷单列，不能视为安全修复完成 |
| 类型化 COM 绑定迁移 | 延期 | 用户确认保留严格 `hr != 0` 失败语义；windows 0.62.2 的公开 typed 方法会接受其他非负 HRESULT。`toast` 保留手写 vtable、VT_LPWSTR 借用和既有 Release 路径，不调用隐藏 vtable API、不新增迁移 features；注释补齐不等于迁移通过 |
| 禁止不稳定功能 | 已满足 | Cargo 保持 Rust 2024/stable，没有 `#![feature(...)]`，未引入 nightly 工具链或 Git 依赖；rustfmt 不稳定选项按规范允许的 stable 例外处理 |
| getter/setter 命名 | 已落实 | `Core` 和 `CoreMode` 查询不使用多余 get 前缀；`AppState::take_child` 表示移出所有权，`set_child` 表示记录句柄；状态整体借用通过 with 访问器而非返回锁守卫 |
| 接受闭包的访问器 | 已落实 | 私有 APP 仅经 `with_app_state` / `with_mut_app_state` 的 FnOnce 访问；状态引用和 MutexGuard 不外泄。设置修改/保存同一可变闭包串行完成，规则集更新只回写当前仍存在的条目 |
| 成套实现预期 API | 已满足 | 无自定义容器 `len`/`is_empty` 或 `get`/`get_mut` 等不完整成套接口；Core 元数据、状态访问和子进程 take/set 按实际职责提供，不添加无消费者的 API |
| 不调用外部 doc(hidden) 项 | 已满足 | 未为 COM 迁移读取 windows crate 隐藏 vtable；保留自行声明的 ABI 布局与公开 API。其他调用使用公开绑定、trait 或标准库接口，未以隐藏 API 绕过迁移延期决定 |
| 谨慎使用 const fn | 已满足 | 自有函数未引入公开 const fn；Core::ALL、命令 ID、超时和上限为实际使用的常量，不为推测性能扩大常量求值承诺 |
| 不滥用非数学运算符重载 | 已满足 | 无自定义 Add/Mul 等非数学重载，路径使用标准 Path::join，格式化使用 format/write |
| 对外 enum 的 non_exhaustive | 条件适用 | AppError、Core、CoreMode、CoreStatus 均属于单 binary 内部接口，不面向跨团队稳定库 API，按适用边界不机械标注 |
| 对外公开字段 struct 的 non_exhaustive | 条件适用 | Settings、AppState、CoreArtifacts 等仅由本应用协调使用；JSON 演进由 serde 默认值/校验处理，不以 non_exhaustive 代替配置兼容策略 |
| 函数 must_use | 已落实 | Core 路径/元数据、模式查询、UTF-16 转换、配置发现、状态查询及更新中的版本/摘要等返回值已审查并标注；已有 Option、Result 或自有 must_use 结果类型的函数不重复。带动作的回调/写入/清理入口按其副作用契约处理，不机械套用纯函数规则 |
| 类型 must_use | 已落实 | FlagGuard、ComGuard、SingleInstanceGuard 提示必须持有守卫；StatusIcons 提示显式释放，CoreStates、MenuSelection 提示消费结果。资源所有权与释放方式保持各自契约，没有用属性替代运行时生命周期要求 |
| 不在 main 前执行运行期代码 | 已满足 | APP/TRAY_HWND 等使用 OnceLock，警告缓存和原子标志使用常量初始化；启动、COM 和日志仍由 app::run 装配。build.rs 只在构建时运行，不是应用启动前构造器 |

## 实现与测试

| 规则 | 最终静态状态 | 当前代码证据或适用理由 |
| --- | --- | --- |
| 不过度使用类型推断 | 已落实 | UTF-16 缓冲区显式 `Vec<u16>`，进程/路径集合按需要标明元素类型，跨函数结果使用具名结构；局部显然的 HashMap、Result 匹配及 FFI 返回值继续合理推断 |
| 模式匹配优于检查后 unwrap | 已满足 | 状态入口用 `let Some(...) else`，系统句柄和文件操作用 `let Ok(...) else` / match，release 选择和配置校验使用模式匹配；源码没有先检查后裸 unwrap 的新增分支 |
| expect 优于 unwrap | 已落实 | build.rs 的 OUT_DIR/资源构建和测试准备、读回、join 结果均有 expect/expect_err 上下文；无裸 `unwrap()` / `unwrap_err()`。保留明确默认或降级语义的 unwrap_or/unwrap_or_else，包括状态中毒转紧急终止 |
| non_exhaustive 匹配不省略已知分支 | 已满足 | 未新增需要穷举的第三方 non_exhaustive 业务枚举；本地 HTTP 测试仅筛选允许的断连 ErrorKind，其余显式返回错误，符合只关注特定情况的例外；自有 Core/CoreMode/清理步骤的关键 match 穷举 |
| 断言提供上下文 | 已落实 | 状态锁释放/保存失败、PID 查询、ZIP 回滚/路径越界及本地 HTTP 超时断言提供原因、路径或错误结果；值比较直观的 assert_eq 保留其自动实际值/期望值诊断，不增加重复文案 |
| 直接循环优于难读组合链 | 已落实 | 托盘配置 `config_items`、进程 stderr 行迭代及更新所需文件链先命名再循环；测试中的相同复杂循环头同步处理。简单 any/all、enumerate 和短局部匹配保留，不将所有迭代器机械展开 |
| 不直接调用 catch_unwind | 已落实 | 源码无 catch_unwind 调用；FlagGuard/中毒测试用工作线程 panic 与 join。panic hook/失败退出只在父进程限时的独立测试子进程内替换，注入四步记录替身，不触发真实 DNS 或核心终止 |
| 拼接和模板替换优先 format/write | 已满足 | 路径/通知/XML 模板使用 format，摘要和日志使用 write；`toast::xml_escape` 的 replace 是字符转义，不是模板拼接。图标 XML 属性未转义的既有缺陷仍另列，不夹带修复 |
| 使用各代码库对应测试框架 | 条件适用 | 保持 Cargo 内联 `#[test]` 与 tempfile；Chromium/Fuchsia 专属测试框架不适用。本轮没有为纯格式和注释机械增加重复实现的测试，复用既有风险覆盖并执行必要回归 |

## 验收边界与未完成项

- 上表完成规范的逐条静态映射；本轮格式、Clippy、84 项测试及双架构 release 构建已通过，实际命令、首次 Clippy 诊断及其修正、产物字节数见实施交接。自动检查不代替系统运行验证。
- typed COM 迁移明确延期，amd64 临时快捷方式属性读写/通知验证也未完成；不得将手写接口的安全注释等同于公开 typed 绑定迁移成功。
- Windows 托盘、模式切换、配置选择、正常退出和真实 DNS 恢复需要可恢复的隔离环境。本轮禁止启动新应用、终止旧实例或核心，以及修改日常环境的 DNS、注册表或计划任务，因此这些实测保持未验证。
- ARM64 交叉构建只证明编译链接，ARM64 原生运行需要相应设备或环境，当前未完成。
- `GetMessageW == -1` 的既有错误分支、安全说明不能完整覆盖的异常路径，以及图标/DC 释放、计划任务临时 XML、通知 XML 转义、TUN 字符串定位等独立缺陷，仍按重构方案单列；本表不将它们记为已修复。
- 正常行为不变量和故障策略以方案、ADR、源码及实际测试为准；后续若必须改变既定语义才能补全安全保证，应先明确该具体依赖，不以“完成规范”为由扩大修复范围。
