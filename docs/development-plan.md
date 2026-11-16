# Ladder 开发计划

更新于 2026-11-17。最新检查见“提交前审查”；最新交互调整见“版本列表每次运行只自动请求一次”。下一阶段为打包与发布。下列产品方向与行为约定为现行规范，后文保留各轮历史验收记录。

版本兼容边界：正式发布前，不兼容 ladder-main 或 ladder-dev 的历史开发版本，不维护这些版本的数据迁移分支。正式发布后，维护已发布版本的升级兼容，保留用户设置、核心配置与待恢复的 DNS 记录。删除开发期兼容代码不等于永久放弃版本兼容；当前版本自身的异常恢复也不属于历史兼容。

DNS 恢复继续保留机器级记录、便携降级，以及两处记录之间的安全衔接。这些是当前版本的故障恢复能力，不随开发期兼容清理删除。

## 已确认的产品与界面方向

- Ladder 是 Windows 上的 sing-box / Xray 轻量启动器。用户通过外部编辑器维护 JSON，应用负责配置选择、核心运行和必要的系统集成。
- 沿用左侧导航与右侧内容的紧凑工具界面，固定为 **sing-box、Xray、日志、软件设置** 四页。“日志”和“软件设置”依次放在侧栏底部。左上角仅显示 Ladder，不显示副标题和软件版本号。
- 主窗口默认客户区与最小客户区统一为 900×600 逻辑像素，四页共用此尺寸约束。
- 每个核心页面只展示本核心的信息，不增加双核总览页或跨页状态面板。核心运行时，在对应侧栏导航项右侧显示绿色小圆点，停止后隐藏。
- 核心主页不显示核心标题、版本提示和“当前配置”卡片。未安装时显示红色加粗的 `点击右侧“管理版本”安装核心`，字号与按钮相同；已安装时显示蓝底白字的“电源图标 + 启动核心”，运行后变为红底白字的“电源图标 + 停止核心”。“管理版本”始终保持蓝底白字。
- 配置列表沿用版本行布局，文件名使用正常字重、不加粗。当前配置置顶，右侧仅显示正常字重的“当前使用”，不提供编辑、切换或删除；其他配置提供“切换 / 编辑 / 删除”。编辑使用与预览版标签相同的黄色 `#d29922`。删除在原按钮内二次确认。移除底部提示及底部操作按钮。目录入口分别命名为“打开核心目录”“打开配置目录”“打开日志目录”。
- 全软件仅保留 14px 主文字、12px 提示文字两档字号，配置与版本列表的“当前使用”均不加粗。
- 在软件设置页提供“跟随系统”“浅色”“深色”。新安装默认跟随 Windows 应用外观。手动选择立即应用到四页，并在下次打开 Ladder 时恢复。
- 主题选择通过颜色区分选中状态，不显示对号，切换时不改变字号或字重。核心版本列表使用“稳定版 / 预览版”标签，分别提供下载、切换与删除。
- 次要状态优先利用同一行的横向空间，避免无必要的副标题和换行。每个版本只显示一行，依次为版本号、“稳定版 / 预览版”标签、完整下载进度等提示，不截断或省略。版本行移除“已下载”提示，上方“已下载 / 全部”筛选保留。
- 每次运行时，两核心各自在首次进入版本管理页时自动获取一次官方列表；之后再次进入、从侧栏返回和托盘恢复都复用已有列表。首次失败不因切页自动重试，完全退出后重置首次请求标记。标题右侧的 24px 蓝底白色刷新图标按钮支持手动重试与更新，获取期间旋转并禁用重复点击。当前使用版本置顶，其余按版本号降序；“已下载 / 全部”分段控件默认选中“全部”，刷新不改变当前筛选。标题下分段控件居左，“打开核心目录”“返回配置页面”依次成组靠右。稳定版标签及侧栏运行圆点在深浅主题均使用亮绿 `#3fb950`，预览版标签均使用亮黄 `#d29922`。进度放在对应版本行内，已安装的当前版本右侧只显示正常字重的“当前使用”；当前版本目录丢失时仍可重新下载。删除在原按钮内二次确认，点击其他位置或按 Esc 取消。
- 先完成页面、交互和状态反馈，再加固后台能力、接入核心更新、完善发布。

## 需要保持的行为

| 行为 | 约定 |
| --- | --- |
| 配置列表 | 当前配置置顶，右侧仅显示正常字重的“当前使用”，无编辑、切换或删除；其他配置提供“切换 / 编辑 / 删除”。不再保留独立浏览选择或底部操作区。 |
| 配置切换 | 先持久化选择，再停止目标核心并立即启动新配置，沿用原“使用此配置”行为。保存失败时维持原选择和原进程；启动失败时保留新选择以便重试。 |
| 配置删除 | 仅删除本核心配置目录中的指定 JSON，需二次确认，当前配置不可删除。 |
| 配置编辑 | 行内“编辑”依次尝试系统 JSON 的 `edit`、`open` 关联，仅无关联时回退记事本，权限等其他错误直接报告；也可通过“打开配置目录”维护源 JSON。Ladder 不格式化或改写源文件，TUN 名称只随机化到运行副本；修改当前配置后，先停止再启动核心以加载变更。 |
| 进程归属 | 只管理 Ladder 自己启动的进程，停止、重启或退出不影响外部启动的同名核心。 |
| DNS 劫持与恢复 | “劫持DNS到本地回环”控制两种核心的启动。任一核心停止时恢复劫持前各物理网卡的 IPv4、IPv6 DNS 并刷新缓存，期间的手动修改会被覆盖。打开 Ladder 和 Windows 开机时均尝试恢复遗留快照，失败记录保留供重试。 |
| 自动启动 | 现有自动启动设置指“随 Ladder 启动核心”，与 Windows 开机启动分别设计、分别表述。 |
| 托盘 | 关闭主窗口隐藏到托盘；显式退出停止托管核心并执行清理。 |
| 诊断 | 展示可操作的错误摘要，不将用户配置、密钥或原始核心输出直接写入应用诊断。 |
| 提示范围 | 运行、配置切换与配置删除提示属于对应核心的配置页；版本下载、切换与删除提示属于对应核心的版本页；设置、DNS 恢复与开机任务提示只在软件设置页，日志故障只在日志页。异步结果保留发起操作的核心和页面，切页不转移或清除其他页面的提示；不保留全局提示横幅。 |

ladder-main 用作行为和失败案例的参考。迁移时保留上述约定，尤其注意旧版按名称终止进程、统一恢复 DHCP 和改写活动配置的差异。

## 开发阶段

| 阶段 | 交付内容 | 完成条件 |
| --- | --- | --- |
| 1．GUI 布局与视觉 | 四页信息层级；统一字体、间距、颜色、按钮、配置行与提示样式 | 两核心页面结构一致；当前配置、状态、操作清楚；浅深色均可读 |
| 2．核心页面交互 | 配置切换、编辑与删除；打开配置目录、启动核心、停止核心 | 切换后立即启动；当前项不提供操作按钮；编辑后可停止并重新启动 |
| 3．状态与桌面体验 | 操作中、核心缺失、配置缺失、空列表及错误状态；键盘、缩放、托盘交互 | 状态与后台一致；键盘可完成主要操作；窄窗口、长文件名、缩放和托盘往返可用 |
| 4．运行与网络加固（已完成） | 权限入口；异常退出的进程治理；DNS 恢复；双核心运行验证 | 真实 Windows 场景验证通过；异常后有明确恢复路径；不干扰外部进程与原网络设置 |
| 5．核心多版本管理 | 发布列表、稳定/预览标签、下载进度、校验、版本共存、切换与删除 | 下载不打断运行；版本隔离；切换保留配置；删除保护当前版本；失败保留可用版本 |
| 6．打包与发布 | CI、amd64/arm64 构建、可复现便携包、首次使用和升级文档 | 干净环境能按文档使用；发布包无个人配置、运行状态和敏感信息 |

阶段 1–3 组成第一个 GUI 里程碑。第一轮只做下面列出的具体任务；完成后再根据实际使用补齐该里程碑的剩余问题。

## 第一轮：紧凑三页界面

**第一轮已完成。** 验收日期为 2026-11-12，结果及回归步骤见下文。

| 任务 | 实现范围 | 验收要求 |
| --- | --- | --- |
| 三页布局 | 统一页标题、状态区、操作区、配置列表和设置项；保持紧凑间距 | 每页重点清楚，不展示另一核心的状态；两种主题布局一致 |
| 通用控件 | 统一按钮、配置行、状态标识、错误提示、悬停、按下、禁用和焦点样式 | 控件状态可辨；仅靠颜色之外仍能读懂运行及错误状态 |
| 操作反馈 | 根据实际动作显示启动中、停止中、切换中、重启中；清楚区分未安装和已停止 | 忙碌时阻止重复运行操作；完成或失败后解除忙碌状态 |
| 配置入口 | 打开本核心配置目录；使用外部编辑器打开所选配置 | 中文及空格路径可用；文件不存在时给出错误；不修改源 JSON |
| 重启入口 | 重启当前已保存配置，供外部编辑后的重新加载使用 | 配置选择保持不变；另一核心及外部同名进程不受影响 |
| 键盘与适配 | 完善现有 Tab / Shift+Tab、焦点提示及键盘激活；检查长文件名与列表滚动 | 主要操作可由键盘完成；焦点与可操作项一致；内容和操作不会被截断 |

第一轮继续连接现有运行模块。核心下载、权限提升、Windows 开机启动和 TUN 残留清理留在后续阶段，不用无实际行为的按钮代替实现。

## 验收与进度记录

每轮在提交说明或本节记录“实际运行的检查、结果、尚未覆盖的场景”。代码已实现与场景已验收分别标记；未运行的检查保持待验收。

### 提交前审查 · 2026-11-17

按现行需求审查配置操作、编辑器关联回退、版本列表布局与请求生命周期、字号及窗口尺寸。修正 README 中仍称进入页面就重新获取列表的旧说明；版本列表保留已确认的单行布局。

格式检查、Clippy（全部 targets、warnings 作为错误）、Debug 构建及 `cargo test --no-run` 通过；[12 项精确过滤测试](../target/review-validation-20261117/safe-test-results.json)及 [17 项隐藏 UI 行为检查](../target/version-ui-check/auto-fetch-once-results.json)全部通过。测试仅使用内存模拟与临时文件，未启动核心或操作本机网络；未重跑完整测试、实际编辑器关联、可见 GUI/DPI 或 Release 验收。

### 版本列表每次运行只自动请求一次 · 2026-11-16

sing-box 和 Xray 分别记录本次运行是否已触发自动请求。首次进入版本页时先记录，再发送请求；后续再次进入、侧栏往返及托盘恢复复用已有状态，不再自动获取。首次请求失败也不因切页重试，已有错误继续保留。手动刷新及加载更多沿用原逻辑，完全退出后下次运行重新允许首次自动请求。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings` 与 Debug 构建通过。纯内存 Runtime 配合实际 UI 操作入口的 [17 项隔离行为检查](../target/version-ui-check/auto-fetch-once-results.json)全部通过，覆盖首次请求、加载中重复进入、成功后的页面往返、两核心独立、异步和同步失败后不自动重试、错误保留、手动刷新及新会话重置。检查运行于单个隐藏无焦点窗口，进程正常退出，未访问真实 API、启动核心或操作本机网络；未执行可见窗口点击与托盘实机回归。

### 版本单行布局与窗口尺寸 · 2026-11-16

版本号、稳定版/预览版标签与下载进度改为同行显示，移除每行“已下载”提示，保留顶部筛选；进度使用完整文本。主页安装指引改为红色加粗。本轮验收时，默认客户区及最小客户区均为 960×640 逻辑像素。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings` 与 Debug 构建通过。在 125% 缩放的纯 UI 隔离窗口检查了[长配置文件名](../target/version-ui-check/evidence/window-960-config-light.png)、[完整设置页](../target/version-ui-check/evidence/window-960-settings-light.png)、日志页按钮、[浅色](../target/version-ui-check/evidence/window-960-install-red-light.png)及[深色安装提示](../target/version-ui-check/evidence/window-960-install-red-dark.png)、[单行完整下载进度](../target/version-ui-check/evidence/window-960-progress-full.png)，以及下载完成后无“已下载”行内提示。向内拖动右下角后，[窗口保持最小尺寸](../target/version-ui-check/evidence/window-960-minimum.png)。

验收窗口已关闭，未启动真实核心或修改本机网络。未复验其他 DPI、Release 构建及最长 64 字节的版本标识。

随后按用户选择将默认客户区与最小客户区调整为 900×600 逻辑像素，`cargo fmt --check` 与 `cargo build --locked --offline` 通过。以上截图和 GUI 验收结果保留为 960×640 的历史记录，900×600 的 GUI 尚未复验。

### 当前配置操作与当前使用字重 · 2026-11-16

当前配置行移除“编辑”，仅保留正常字重的“当前使用”；版本列表的“当前使用”同步取消加粗。格式检查、Clippy（全部 targets、warnings 作为错误）及 Debug 构建通过；660×440、125% 缩放的纯 UI 隔离窗口核对了[当前配置无编辑按钮](../target/version-ui-check/evidence/current-config-no-edit.png)及[版本当前使用正常字重](../target/version-ui-check/evidence/current-version-normal-weight.png)，未启动真实核心或操作网络。

### 启停按钮与配置文件名字重 · 2026-11-16

两核心启停按钮改为电源图标配“启动核心 / 停止核心”；配置文件名恢复正常字重，“当前使用”标识仍加粗。格式检查、Clippy（全部 targets、warnings 作为错误）、Debug 构建及实际 `core_page` 的 [16 例隐藏构造检查](../target/version-ui-check/power-render-green.json)通过；660×440、125% 缩放的纯 UI 隔离窗口核对了前置电源图标、[蓝白启动与正常字重文件名](../target/version-ui-check/evidence/power-start-normal-files.png)，以及点击后的[红白停止](../target/version-ui-check/evidence/power-stop-normal-files.png)，未运行真实核心或操作网络。

### 配置行编辑与切换启动更正 · 2026-11-16

用户明确选择行内“切换 / 编辑 / 删除”：编辑为预览版同款黄色 `#d29922`，按 `edit → open → 记事本` 查找可用关联，仅无关联时回退，权限等其他错误直接报告；当前配置保留“当前使用”和“编辑”，不提供切换或删除。切换配置立即启动，沿用原“使用此配置”的行为，取代上一轮暂定的“只保存并停止、手动连接”。

格式检查、`cargo clippy --locked --offline --all-targets -- -D warnings`、Debug 构建及 `cargo test --no-run` 通过；精确过滤的 6 项安全测试通过，覆盖配置操作、操作排队及桌面错误路径。两核心 × 深浅主题 × 四种连接状态的实际 `core_page` [构造矩阵](../target/version-ui-check/editor-render-green.json)共 16 例通过，进程正常退出。纯 UI 隔离窗口在 660×440、125% 缩放下核对了[浅色](../target/version-ui-check/evidence/config-editor-light.png)、[深色](../target/version-ui-check/evidence/config-editor-dark.png)三按钮、当前行编辑及长文件换行，并验证[切换后立即运行并显示红色停止](../target/version-ui-check/evidence/config-switch-start.png)。

验收窗口已关闭。未调用真实编辑器、修改文件关联、启动真实核心或操作网络；实际关联程序与记事本启动链尚未实机验收。以下上一轮记录保留为当时的历史结果。

### 核心主页与版本刷新统一 · 2026-11-16

本轮同步调整 sing-box 和 Xray：版本页进入即刷新、默认显示全部、刷新图标旋转；核心主页简化为安装指引或连接/停止入口，配置列表改为行内切换与删除；统一两档字号及三个目录按钮名称。切换配置只保存并停止目标核心，之后手动连接。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、Debug 构建及 `cargo test --no-run` 通过。9 项精确过滤的安全测试通过，覆盖配置选择与删除、操作排队、主题、桌面入口及版本列表合并与排序。

隐藏检查调用实际 `core_page`，覆盖两核心 × 深浅主题 × 停止、运行、连接中、停止中，共 16 例，全部通过且进程正常退出。修复前的 4 个运行态均复现 `hover style already set`，现将按钮颜色一次传入，避免重复注册悬停样式。记录见[修复前](../target/version-ui-check/connection-render-red.json)与[修复后](../target/version-ui-check/connection-render-green.json)。

纯 UI 隔离窗口在 660×440 最小客户区、125% 缩放下检查了两核心主页、长文件名及[未安装指引](../target/version-ui-check/evidence/core-uninstalled-light.png)；版本默认全部、进入自动刷新、[旋转过程](../target/version-ui-check/evidence/auto-refresh-spin-1.png)及[刷新完成](../target/version-ui-check/evidence/auto-refresh-complete.png)；[连接后的红色停止按钮](../target/version-ui-check/evidence/core-running-light.png)、[配置切换后停止并置顶](../target/version-ui-check/evidence/config-switched-light.png)、[删除确认](../target/version-ui-check/evidence/config-delete-confirm.png)与[Esc 取消](../target/version-ui-check/evidence/config-delete-cancel.png)；[深色运行](../target/version-ui-check/evidence/core-running-dark.png)与[停止](../target/version-ui-check/evidence/core-stopped-dark.png)，以及设置、日志两档字号和[日志目录按钮](../target/version-ui-check/evidence/log-folder-dark.png)。

验收窗口已关闭，无测试进程残留。隔离界面使用内存状态，未运行真实核心、实际 HTTP 或进程测试，未操作本机 DNS、TUN 或网络；完整 DPI 矩阵及 Release 构建未复验。后续进程测试应在 Windows 沙盒运行。以下为此前各轮记录，不代表本轮已复验。

### 自动检查

- [x] `cargo fmt --check` 和 `cargo clippy --locked --all-targets -- -D warnings` 通过。
- [x] `cargo test --locked` 通过。13 个用例覆盖配置持久化、错误摘要、DNS 回滚、进程归属及桌面文件入口。操作状态用例还验证了四种操作进行时会拒绝重复请求，成功、失败或工作线程断开后均解除忙碌状态。
- [x] Windows 进程回归用例验证重启保留已保存的配置，只替换目标进程，另一核心及外部启动的同名进程继续运行。

### Windows GUI 验收结果

Windows 10 22H2 桌面实际缩放为 125%。常规构建在隔离目录中运行，使用 20 份 sing-box 配置、2 份 Xray 配置和两个可启动、可按指定配置失败的模拟核心。窗口控件用 Computer Use 文本接口核对，画面由 `capture-window.ps1` 按程序路径和完整窗口标题截取。

| 场景 | 实际结果 |
| --- | --- |
| 三页、主题与缩放 | sing-box、Xray、软件设置分别检查浅色和深色，覆盖 820×510 默认客户区、660×440 最小客户区，以及 100%、125%、150%、200% 四档缩放，共 48 组。两核心各自只显示本核心状态，列表底部操作和三项设置可见。隔离构建复用同一套界面代码，指定浅深色并向窗口发送 DPI 变更消息，逐组核对 GPUI 实际缩放和客户区尺寸。 |
| 空目录与配置变化 | 首次启动显示未安装和空目录指引。放入模拟核心和 JSON 后列表自动更新。移走当前配置后，数量从 20 变为 19，界面提示重新选择且无法启动。放回同一文件后数量恢复，启动按钮重新可用。 |
| 浏览、使用与文件入口 | 鼠标、Enter 和空格均可选中配置，浏览保留当前配置和运行状态。“使用此配置”保存选择并启动核心。“打开目录”通过资源管理器的实际目录核对，“编辑”通过记事本窗口标题和进程参数核对，均指向含中文配置名的测试目录或文件。 |
| 启停、重启与失败恢复 | 实际触发启动、停止、重启和切换。为稳定观察瞬态，在隔离构建中延迟处理请求，四种操作均显示对应文字并禁用重复操作。模拟核心以退出码 42 结束时，界面显示权限错误摘要，保留已保存的选择。改用正常配置后，界面和子进程均恢复运行。 |
| 双核心与源文件 | 两个模拟核心同时运行时，从外部修改 sing-box 测试 JSON，再点击重启，sing-box 的 PID 由 6004 变为 18212，Xray 保持 3652。新进程使用原配置路径，外部修改后的文件哈希保持不变。还原测试改动后，22 份源文件的 SHA-256 全部符合初始内容。 |
| 键盘和长文件名 | 20 项列表从首项用 Tab 到第 6 项并按 Enter，长中文名完整换行。继续到第 20 项用空格选中，再用 Shift+Tab 返回首项。焦点框随列表滚动，当前配置保持不变。从侧栏按 Tab 会跳过禁用按钮，用 Enter 或空格可依次启动、重启、停止，已核对进程变化。125% 窄窗口与 200% 深色最小窗口均复测了点击底部长行，文件名和页脚按钮同时完整可见。 |
| 设置与托盘 | 在设置页开启和关闭“随 Ladder 启动 Xray”，两次均核对已写入状态文件。关闭主窗口后进程保持运行。按测试进程注册的图标位置核对系统通知区域，鼠标左击恢复同一窗口，右击打开菜单。点击“退出并终止”后，主程序和两个托管核心均已退出。 |

### 本轮修复与下次回归步骤

| 实机发现 | 修复 | 复测方法 |
| --- | --- | --- |
| 停止核心或重新打开已有配置时，窗口进程崩溃 | 启动按钮只注册一次悬停、按下和焦点样式，消除 GPUI 的重复注册断言 | 保存配置后重新打开，依次启动、重启、停止、再启动，检查窗口、按钮状态及子进程。 |
| Tab 跳过全部配置行 | 给每个文件的焦点句柄显式设置 Tab 停靠和排序 | 从首项正向遍历至末项，再反向返回，以 Enter 和空格选择，核对焦点框和当前配置标记。 |
| 点击列表底部的长文件名后，换出的第二行被页脚遮住 | 选中行时按展开后的行高自动滚动，使该行完整可见 | 在窄窗口将长行移到列表底部，分别用鼠标和 Enter 选择，核对文件名末尾及页脚操作。 |
| 运行期间移走配置目录后，点击重启会先中断原进程 | 扫描失败时禁用启动、重启和使用配置。后台在停止原进程前复核目标配置是否可读 | 保留原进程并移走目录，尝试重启或切换配置，核对 PID 和已保存选择。放回目录后重启，确认此时才更换 PID。 |

本机保留了 [125% 窄窗口长行截图](../target/gui-check/accept-22c5f6ea/evidence/29-min-long-click.png)、[200% 深色长行复测截图](../target/gui-matrix/evidence-20261112-020124/scroll-200-fixed-click.png)、[核心启动失败截图](../target/gui-check/accept-22c5f6ea/evidence/21-fail-retry.png)、[48 组截图索引](../target/gui-matrix/evidence-20261112-020124/captures.csv)和[四种操作的进程记录](../target/gui-matrix/evidence-20261112-020124/busy-check.csv)。这些产物位于 Git 忽略的 `target/` 目录，清理构建目录会一并移除。

真实核心的网络连通性、TUN、网卡 DNS 变更与系统重启恢复的验收结果见下文阶段 4 记录。

### 第一轮补充验收 · 软件设置页主题选项

2026-11-12 在 Windows 10 22H2（19045）、125% 缩放下，使用常规调试构建和独立状态目录实测。入口位于 **软件设置 → 主题**。鼠标点击或键盘选择后，三页和原生标题栏立即采用新外观。选择保存在便携目录的 `ladder-state.json` 的 `settings.theme` 字段。

| 检查 | 验收结果 |
| --- | --- |
| 首次使用与旧配置 | 以缺少主题字段的旧状态启动，设置页选中“跟随系统”，与 Windows 当前的浅色应用模式一致。更改主题后，两个核心原有的配置选择均保留。自动测试另覆盖全新目录的默认值。 |
| 即时切换 | 在同一窗口从浅色切到深色，再切回浅色。设置页、sing-box 和 Xray 的正文与原生标题栏配色一致。实测发现 Windows 10 曾在客户区变色后保留旧标题栏，已修复窗口边框重绘并双向复测。 |
| 重启与托盘 | “浅色”“深色”“跟随系统”分别通过托盘显式退出、重新启动并打开设置核对。三次均恢复上次勾选的选项。深色模式下隐藏到托盘并恢复，仍保持深色。 |
| 键盘与最小窗口 | Tab 可移到主题选项，Enter 和空格均能切换。660×440 最小客户区下，三个选项、勾选标记和原有设置完整可见。 |
| 保存失败与恢复 | 临时锁定隔离状态文件后切换主题，界面提示保存失败并恢复最后成功保存的深色。释放文件锁，按空格重试后，提示消失且“跟随系统”成功保存。 |

`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 及全部 16 项测试通过。本机实际验证了从固定深色返回 Windows 当前的浅色。系统外观变化时的浅、深映射及固定主题的优先级由自动测试覆盖，实机期间保留了用户的 Windows 个性化设置。

本机验收资料包括[深色设置页](../target/theme-acceptance/run-6d32801f/evidence/22-dark-fixed.png)、[固定浅色设置页](../target/theme-acceptance/run-6d32801f/evidence/25-light-fixed.png)、[最小窗口](../target/theme-acceptance/run-6d32801f/evidence/30-minimum-dark.png)和[客户区与标题栏配色核对记录](../target/theme-acceptance/run-6d32801f/evidence/verified-colors.csv)。

### 阶段 4 · 运行与网络加固

以下为阶段 4 原版本的验收记录。当前 DNS 行为以上表为准。

- **权限**：桌面程序的清单要求 `requireAdministrator`。Windows 在进入 Ladder 前处理提权，启动核心时沿用该权限。已用 Windows SDK 提取并检查嵌入清单，同时核对原有 PerMonitorV2 和 Common Controls 设置。普通权限直接创建产品进程返回 Windows 错误 740。自动测试仍可用普通权限运行。
- **进程归属**：每个核心在创建时加入自己的 Windows Job。强制结束 Ladder 时由系统清理整个托管进程树。正常停止或核心自行退出时，Ladder 等待其后代结束，再处理输出和 DNS。Windows 进程用例覆盖子进程和孙进程、两核心互不影响及外部同名进程存活。
- **双核心连通性**：使用官方 sing-box 1.14.4 和 Xray 26.11.9，各自通过 SOCKS5 请求回环 HTTP 服务并校验响应。定向重启、核心意外退出后再启动、正常退出和强制结束宿主均已在独立测试目录验证。外部 Xray 保持服务，两份配置的哈希保持一致。
- **真实 TUN 与 DNS**：在 Windows 10 沙盒中同时启动两个核心，Xray 创建 Wintun 接口，Ladder 接管来宾网卡的 IPv4 和 IPv6 DNS。意外终止 Xray 后，IPv4 恢复原静态值，IPv6 恢复自动获取，sing-box 继续运行。再启动后手动修改 IPv4 DNS 并强制结束 Ladder，两托管进程均退出。下次打开 Ladder 保留手动设置的 IPv4 并恢复 IPv6。网络测试只操作沙盒网卡。
- **恢复失败**：待恢复的网卡按接口和地址族写入 `dns-recovery.json`。恢复失败会显示原因和“重试恢复 DNS”。两个核心均已停止时可保留记录退出，下次启动自动重试。单测覆盖日志写入中断、部分恢复、外部修改、损坏记录、日志无法更新和重试。

**Release 界面验收**：安装 x64 Visual C++ v14 运行库后，使用本轮 Release 产物在 Windows 10 沙盒打开 Ladder。已核对主窗口的标题、窗口类、响应状态和可执行文件哈希。用损坏的恢复记录和“启动时隐藏主窗口”设置打开时，主窗口显示。修好测试记录后按 Tab、Enter 激活重试，恢复成功并清除记录。调试版在构建机以外运行需要 GPUI 源码目录里的三份着色器；在同一沙盒补入文件后调试版可打开，移除后 Release 仍可独立打开。

`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、全部 23 个测试及 `cargo build --release --locked` 通过。

2026-11-12 用户在 Windows 上手动验证了 UAC 确认与取消，并分别验证了两种退出顺序：

- Xray 先退出时，仍在运行的 Ladder 恢复网卡原有 DNS 设置，随后删除 `dns-recovery.json`。
- Ladder 主进程先被强制终止时，恢复记录保留在磁盘上，下次打开 Ladder 时执行恢复。

2026-11-12 用户完成整机重启手测。保留 `dns-recovery.json` 重启 Windows，第一次打开 Ladder 后，网卡 DNS 恢复原值，恢复文件随即删除。阶段 4 验收通过。

### DNS 开关与开机恢复验收

新版本通过 41 项自动化测试、Clippy 和 Release 构建。回归用例覆盖了旧快照被占用或损坏后，桌面程序与开机任务接续恢复的顺序。Windows 10 沙盒用测试核心和来宾网卡验证了 sing-box、普通 Xray 单独或同时启动时的双栈回环设置。双核运行时让 Xray 退出，sing-box 继续运行，网卡恢复静态 IPv4 与自动 IPv6，测试期间手动写入的地址也被覆盖。

沙盒预置的同名任务成功替换。导出的任务定义包含开机触发器和 SYSTEM 身份。在沙盒内用 `Start-ScheduledTask -TaskName Reset_DNS_On_Boot` 按需执行任务，核心运行时保持回环，强制结束 Ladder 后恢复静态 IPv4 与自动 IPv6，快照清除后保留新手填的 IPv4。

升级验收分别使用被占用和损坏的便携快照。Ladder 先按机器记录还原网卡，随后手动设置新的 IPv4。运行 SYSTEM 任务并重新打开 Ladder 后，新值仍然生效。另一次验收故意使机器恢复目录校验失败，关闭劫持的普通 Xray 仍可启动，修复目录后成功恢复。沙盒 14 项检查通过，5 次任务退出码均为 0，本轮未重启整机。

### 阶段 5 · 核心安装与更新

以下为原覆盖式更新的历史验收记录，已由后文“核心多版本管理”替代，不再使用通道选择与 `.old` 轮换。

2026-11-15 更新：两个核心各自选择并记住“稳定版 / 预览版”，默认稳定版。预览通道允许官方预发行版及更新的稳定版，按版本号比较，不自动降级。下载与校验期间保留核心运行，真正替换前停止目标核心并等待退出；**替换后保持停止，由用户手动启动**。原来的配置选择保留，不改写 JSON，另一核心不停止。界面沿用三页，在各核心页面提供版本、通道选择、安装或检查更新、进度及失败重试。

稳定通道使用官方 GitHub Release API 的 `/releases/latest`，预览通道从 `/releases?per_page=30` 返回的发布列表中选择当前架构可用的最高版本；两者都校验资产地址、大小和完整 SHA-256。Windows amd64/arm64 按 Ladder 构建架构选择。归档只提取核心 EXE、随附 DLL 和 Xray 的地理资源；全部暂存并通过版本验证后才交回核心管理线程替换。运行操作与后台下载分离，同一核心不重复下载，退出后迟到的下载结果不会应用。手动放入或更换 EXE 也会触发后台版本检测。版本探针与代理核心共用 Windows Job 归属，损坏 PE 文件提前拒绝，超时和宿主强退均清理探针进程树。

更新网络沿用 `ureq` 的环境代理设置（HTTP/HTTPS 代理及 `NO_PROXY`，未启用 SOCKS 或 Windows 系统代理特性）；无有效环境代理时直连，仍受 TUN/VPN 系统路由影响。安装成功后，各核心目录保留上一版的 `原文件名.old`，直到下一次实际安装前清理并轮换；检查、下载和校验阶段不触碰这些备份。

| 验证 | 结果与范围 |
| --- | --- |
| 自动检查 | `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、Release 构建通过。`cargo test --locked`：58 项通过，1 项联网验收默认忽略；该联网用例已另外显式运行并通过。 |
| 网络与归档失败 | 本地 HTTP 服务覆盖中断、错误哈希、重定向、HTTP 错误、超时和重试。ZIP 用例覆盖危险路径、重复文件、缺少 DLL 和完整暂存；失败不修改已安装核心。 |
| 替换与运行 | Windows 文件锁用例验证中途失败回滚、删除本次新增文件、无法恢复时保留备份。真实测试子进程验证下载期间可启停、替换只停止目标核心、配置与自定义文件保留、安装后保持停止、退出后不安装。 |
| 官方资产与首次安装 | 四份 amd64/arm64 官方 ZIP 均下载并核对 SHA-256 和文件布局。显式联网用例经产品 HTTP 实现下载 sing-box 1.14.4、Xray 26.11.9，完成暂存验证、安装、本地版本读取和再次检查无需更新。amd64 核心只执行版本命令，未启动代理服务。 |
| Windows GUI | 普通权限、独立状态目录、便携 DNS 的隔离构建，界面及运行逻辑来自本次源码。660×440 客户区、125% 缩放实测深浅色；实际点击安装 sing-box，观察下载进度并切换设置页，安装后显示版本且保持停止。Tab 可到更新按钮，空格能触发检查；正常状态的配置列表和底部操作完整可见。另放入损坏的 Xray 验证错误摘要，再从更新入口重新安装，错误消失并显示正确版本，仍保持停止。 |

本轮通道与备份调整：`cargo test --locked -- --include-ignored` 的 64 项测试全部通过，覆盖两核心通道独立保存、旧设置默认值、保存失败、更新中锁定、预发布排序、`.old` 保留与轮换及 Windows 文件占用回滚。`cargo fmt --check`、Clippy 和 Release 构建通过。预览列表缩减为 30 条后，另外重跑 `official_preview_install`，通过 sing-box `1.15.0-alpha.13` 和 Xray `26.11.9` 的实际下载、校验、安装、版本读取与重复检查。新的隔离 GUI 构建成功打开；用户按 Esc 停止 Computer Use，本轮新增通道控件的视觉和键盘验收未完成。未重新执行此前的完整 GUI/DPI 矩阵。

随后移除版本通道和主题选项的对号及选中加粗，通道字号保持 13px。对全部待提交更新改动分别进行实现规范和需求审查，未发现待修复问题；`cargo test --locked` 的 62 项离线测试、格式检查、Clippy 和 Release 构建通过，两项联网安装用例本轮未重跑。为避开正在使用的 ladder-main，新建 `target/channel-ui-check/` GUI 副本，使用独立实例锁、目录与模拟 DNS，移除测试副本的开机恢复入口，不安装或启动真实核心。获准恢复 Computer Use 后成功打开该副本，但指定截图脚本无法匹配前台窗口，原生截图重试仍超时，视觉和键盘实测保持未完成。测试前后的 ladder.exe、Xray 进程 ID 与启动时间一致。

本机资产证据位于 [asset-evidence.json](../target/stage5-research/asset-evidence.json)，隔离 GUI 工程和状态位于 `target/stage5-gui/`，均被 Git 忽略。

**验收边界**：ARM64 仅校验包内容与哈希，未在 ARM64 Windows 上运行。未重复上一阶段的全部 DPI 矩阵、真实 DNS/TUN 或整机重启测试。安装事务覆盖普通 I/O 失败的自动回滚；替换中被强杀或断电时保留核心目录中的 `.old`，重新打开不会自动恢复该事务，需要手动恢复。暂存清理失败会保留路径提示，不把已成功安装的版本回滚为不完整旧版。此前版本遗留在 `.ladder-update-*/backup` 的备份仍需手动检查处理。

### 开发期设置兼容清理 · 2026-11-16

删除旧状态缺字段时自动补默认值的逻辑。首次启动仍使用正常默认值，当前完整格式支持保存与重开；缺字段的历史状态报错且不改写原文件。DNS 的机器级记录、便携降级和两处记录衔接全部保留，核心更新的 `.old` 备份行为不变。

`cargo test --locked --offline` 的 63 项测试通过，2 项官方联网安装测试保持忽略；格式检查、Clippy 与独立审查通过。测试使用临时目录、模拟 DNS 和测试核心，未修改实际网卡或计划任务；测试前后的 ladder-main 与 Xray 进程 ID、启动时间一致。

### 应用日志与独立查看窗口（历史方案）· 2026-11-16

本节保留最初独立日志窗口的实现和验收记录。日志入口随后调整为主窗口侧栏页面，当前方案与本轮验收见下一节；下列独立窗口截图和检查不代表侧栏版本已验收。

按本轮确认沿用 ladder-main 的会话文件行为：程序目录 `app.log` 每次启动覆盖，不保留历史、不轮转。文件最多 2 MiB，达到上限后停止文件写入并提示，内存继续保留最近 500 条。文件不可写时保留界面日志，不阻断核心管理或 DNS 恢复。开机恢复取得快照锁后，在机器恢复目录覆盖自己的 `app.log`。

日志涵盖启动退出、核心操作及意外退出、DNS、核心更新阶段和设置保存；高频轮询的重复故障去重，核心输出和异常文本只记录安全摘要。新增管道读取失败诊断及 panic 源码位置记录，不记录配置、环境变量、核心原文或 panic payload。

当时在软件设置标题右侧新增“查看日志”，打开可复用的独立窗口，提供暂停/继续、复制、打开目录及键盘翻页，沿用主窗口主题和三页导航。

自动检查：`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo test --locked --offline`（70 项通过，2 项官方联网安装验收保持忽略）和 Release 构建通过。新增测试覆盖会话覆盖、大小上限、并发整行写入、文件故障降级、Unicode/控制字符、独立子进程 panic 隐私及核心/DNS 摘要不泄漏测试值；独立代码审查未发现待修复问题。

GUI 验收使用 `target/logging-ui-check/` 下的独立状态目录、实例锁和模拟 DNS 副本。在交互用户上下文运行指定截图脚本后，125% 缩放下完成普通尺寸和 600×360 最小客户区验收：浅深色及标题栏同步、500 条日志、暂停时文件继续记录而窗口及复制内容保持冻结、恢复后显示最新记录并滚到末尾、Shift+Tab/Enter、Home/PageDown、最小化后复用原窗口、关闭后重开均通过。中文、无空格英文和 8 KiB 长行正常换行；复制得到 500 条，最长记录为 8191 字节并含截断标记。截图见[浅色日志](../target/logging-ui-check/12516b787c3c4b6ca4ec58129b206f09/evidence/01-light-logs.png)、[深色暂停](../target/logging-ui-check/12516b787c3c4b6ca4ec58129b206f09/evidence/02-dark-paused.png)和[最小尺寸长行](../target/logging-ui-check/narrow-cc01e8d8a2714144a792069e946ec7d8/evidence/03-narrow-8k-wrap.png)。

实测发现同步调用 ShellExecuteW 打开目录时会重入 GPUI 定时刷新，触发 App 可变借用冲突并退出。日志目录、配置目录和编辑配置入口均改为前台任务，在点击回调结束后执行 Shell 操作，完成后才更新视图。实际回归成功打开两个目录及验收 JSON 的记事本，程序保持运行且无新增 panic；没有引入浅层模拟测试来代替 Windows 消息循环回归。修复后的格式检查、Clippy、70 项离线测试及 Release 构建再次通过。测试副本已关闭，原有 ladder-main 与 Xray 的进程 ID、启动时间保持一致；未修改真实 DNS 或计划任务，未重复全套 DPI 与整机重启验收。

### 应用日志移入侧栏 · 2026-11-16

本节记录首次侧栏实现与验收，警告配色和时间的后续调整见下一节。

按本轮决定，将“应用日志”放入主窗口左侧导航，与 sing-box、Xray 并列，软件设置仍位于侧栏底部。移除设置页“查看日志”入口和独立日志窗口，页面沿用主窗口主题与标题栏。

保留最近 500 条日志、暂停/继续刷新、复制当前显示内容、打开目录和键盘翻页。切换页面保留日志视图状态；暂停时后台仍记录日志，显示与复制内容保持冻结，继续刷新后显示最新记录并滚到末尾。`ERROR` 整行沿用适配浅深色主题的红色，`INFO`、`WARN` 使用普通正文颜色，不增加 JSON 语法或 ANSI 终端颜色高亮。

本轮 `cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings` 通过；`cargo test --locked --offline` 为 70 项通过、2 项联网安装测试忽略。退出旧调试版后，清理本项目的调试编译缓存并完成 `cargo build --locked`，12 个运行状态、备份、核心及配置文件的 SHA-256 保持一致。隔离验收构建通过，使用独立目录、实例锁、模拟 DNS 和 `asInvoker` 清单；其 `ui.rs`、`log_view.rs` 与本轮源码的 SHA-256 一致。隔离副本复用 `target` 时必须使用独立的 package 和 lib 名，仅修改 bin 名会污染正常构建的脚本与库缓存。

Windows 125% 缩放下完成 820×510 普通客户区与 660×440 最小客户区的浅深色验收：四页切换只使用主窗口，`ERROR` 行显示红色；暂停后跨 Xray、软件设置切页，日志内容和滚动位置保持，后台日志继续追加。复制得到 500 条、最长记录 8191 字节，暂停前后复制内容哈希一致，恢复刷新后显示最新记录并滚到末尾。Tab 从 Xray 移到日志并用 Enter 打开，以及 Home、End、PageUp 均通过；8 KiB 无空格长行正常换行，无横向溢出。打开目录指向隔离应用目录，程序持续运行且无 panic。截图见[浅色侧栏](../target/log-sidebar-check/e879cc6ea2c3/evidence/01-light-sidebar.png)、[深色暂停](../target/log-sidebar-check/e879cc6ea2c3/evidence/02-dark-paused.png)、[最小浅色窗口](../target/log-sidebar-check/e879cc6ea2c3/evidence/03-minimum-light.png)和[最小窗口长行](../target/log-sidebar-check/e879cc6ea2c3/evidence/04-minimum-long-line.png)。

本轮未重复全套 DPI、真实 DNS 或 Release 构建验收。隔离进程已停止，原有 Ladder 进程（7648、14060）未操作。上述为侧栏版本的独立验收，未沿用历史独立窗口的检查结果。

### 日志警告配色与本机时间 · 2026-11-16

按后续要求，`WARN` 日志整行改为黄色，`ERROR` 保持红色，`INFO` 保持普通正文颜色。浅色主题使用 `#8a6500`，在白色日志面板上的对比度约为 5.33:1；深色主题使用 `#f2cc60`，在 `#24272c` 面板上的对比度约为 9.69:1。保留级别文字，颜色作为辅助识别。

日志时间改为 Windows 本机时间，使用 `GetLocalTime` 按系统时区记录为 `YYYY-MM-DD HH:mm:ss.SSS`，移除日志页右上角的 UTC 标签。

本次 `cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo test --locked --offline`（70 项通过，2 项联网安装测试忽略）和 `cargo build --locked` 通过。独立 package/lib/bin 的隔离验收副本构建成功，三个修改源码与主仓 SHA-256 一致。

用户授权重试后重新初始化 Computer Use，在 Windows 125% 缩放下完成浅色、深色日志页实机验收：`WARN` 分别为深黄色和亮黄色，`ERROR` 为红色，`INFO` 为普通正文色；INFO 正文含 `[WARN]`、`[ERROR]` 时不误染，WARN 正文含 `[ERROR]` 时仍保持黄色。两主题右上角均只显示条数，日志时间显示为本机的 `YYYY-MM-DD HH:mm:ss.SSS`，无 `T`、`Z` 或 UTC 标签。分别将最新日志时间与 Windows 本机时钟比较，记录距当前时间为 2.134 秒和 1.518 秒，符合验收副本每 3 秒生成一组日志的间隔。截图见[浅色日志与本机时间](../target/log-colors-check/fa6c1349d6e0/evidence/01-light-local-time.png)和[深色日志与本机时间](../target/log-colors-check/fa6c1349d6e0/evidence/02-dark-local-time.png)。本轮仅验收显示与时间，使用模拟 DNS，未操作真实核心或网卡。

提交前复审补齐两处细节：日志文件故障或达到上限时，相同提示不再同时出现在全局区域和日志页；其他错误存在时仍保留日志警告。README 的更新按钮名称与界面“检查更新”统一。修复后 Clippy、70 项离线测试（2 项联网测试忽略）及调试构建再次通过，规范与需求两轴复审均无待修复问题。

### TUN 名称与残留处理 · 2026-11-16

参考 ladder-main 补齐 TUN 生命周期：两种核心的每个 TUN 入站在启动时使用新的 `tun-` 随机名称。通过同配置目录的 `.ladder-tun-*.json` 运行副本实现，源文件、资源目录和核心工作目录保持原有规则。运行副本不显示在配置列表中，随托管核心结束回收；强杀遗留在下次打开时清理。支持已有注释和尾逗号配置。

用户明确选择沿用 ladder-main 的清理范围：所有孤立或异常 WinTUN 设备，以及描述以 `tun-` 开头的网络历史记录，可涉及其他 VPN 的残留。另一托管核心使用的接口和健康设备受保护，网络记录排除系统现存接口，查询失败时跳过，清理失败记警告。保留现有 Job 进程树管理和 DNS 快照恢复，先恢复 DNS 再执行设备清理，不引入旧版按进程名终止或统一恢复 DHCP 的行为。

本轮 `cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo test --locked --offline`（78 项通过，2 项联网安装测试忽略）和 `cargo build --locked --offline` 通过。新增测试覆盖两种核心的多 TUN 随机化、注释与缺失字段、源文件字节保持、双核心独立运行、重启更名、核心启动失败与异常退出、正常停止和退出时回收副本、下次启动清除遗留副本，以及广泛清理范围和健康设备保护。独立复审确认先恢复 DNS 再清理，网络记录删除前重新获取接口列表。

单元测试隔离系统清理，设备移除和网络注册表删除另在 Windows 10 企业版沙盒（10.0.19041）实测。使用官方 sing-box 1.14.4、Xray 26.11.9 和随附 WinTUN，13 项沙盒检查全部通过；所有设备、DNS 和注册表改动均在来宾内执行。

- **随机化与双核心**：sing-box 的两个 TUN 入站和 Xray 的一个入站均创建独立的随机接口。重启 sing-box 后两个名字变化、旧副本删除，Xray 的 PID 和接口保持；两份源配置的 SHA-256 在整轮测试前后完全一致。
- **真实设备清理**：独立进程创建并禁用真实 WinTUN，强杀创建者留下无所有者节点。清理前确认普通定位失败、phantom 定位成功，且 ALL 设备枚举和 Enum 注册表仍有该实例；清理后四处均无该实例。外部设备名不含 `tun-`，验证了用户选择的广泛范围。健康对照设备保留，显式保护名称的残留保留，移除保护后可清理。
- **网络历史记录**：在 Profiles 和 Signatures\Unmanaged 两处分支各构造陈旧 `tun-`、活跃接口同名和无关记录。陈旧项删除，活跃及无关项保留；释放活跃接口后，相应记录在下一次清理中删除。
- **异常与恢复**：强杀 Xray 后恢复来宾网卡的静态 IPv4 DNS 与自动 IPv6 DNS，随后回收设备和运行副本，sing-box 继续运行。强杀管理进程后两个核心均由 Job 终止；再次打开恢复 DNS 并清除实际遗留的两份运行副本。真实核心配置错误、正常停止和正常退出同样回收副本。
- **Release 程序**：`cargo build --release --locked --offline` 通过。实际 Release `ladder.exe` 在沙盒自动启动双核心并生成三个 TUN，强杀后再次打开清除三台残留设备和两处分支的历史记录；源配置保持不变。Release SHA-256 为 `6F610A6E3804EADEA0EA245A7EF476DAD34CDA9F924843C3B907F340CCB0C9F3`。

细粒度启停由独立验收驱动调用生产 Runtime，10 份编入驱动的生产源码与仓库 SHA-256 一致，使用非 `cfg(test)` 的真实设备及 DNS 路径。首轮夹具因仍持有创建句柄而被系统重新枚举，后改为强杀独立创建者；另修正了验收脚本用 PowerShell 5.1 默认编码读取中文 UTF-8 响应的问题。修正后从头重跑全部 13 项，没有跳过清理用例，也未为验收修改产品逻辑。

证据见[完整结果](../target/tun-sandbox-check/run-e091a53e257b4b98af65c0e4f87c3559/output/result.json)、[设备与注册表观测](../target/tun-sandbox-check/run-e091a53e257b4b98af65c0e4f87c3559/output/cleanup-observations.jsonl)、[源码哈希核对](../target/tun-sandbox-check/run-e091a53e257b4b98af65c0e4f87c3559/output/driver-source-hashes.json)和[Release 恢复日志](../target/tun-sandbox-check/run-e091a53e257b4b98af65c0e4f87c3559/output/tun-qa-product.log)。本轮验收范围为 TUN 生命周期，未重复 ARM64 构建或 GUI 视觉验收。

提交前复审补齐 Xray 的大小写识别：`Inbounds`、`Protocol`、`Settings`、`Name` 等写法及 `TUN` 协议值均能生成随机名称，只更新运行副本中的原名称字段，保留其他设置和源文件。补充回归先在 Windows 沙盒复现失败，修复后完整测试为 79 项通过、2 项联网测试忽略；格式检查、Clippy 和调试构建通过，规范与需求复审无待修复问题。本次仅修正名称随机化所需的大小写匹配。

同一修复另在 Windows 沙盒使用官方 Xray 26.11.9 完成 7 组配置校验：6 组字段或协议大小写变体，以及 1 组非 TUN 配置。原配置和运行副本均通过核心检查；生成名称被核心实际读取，MTU、gateway 和其他字段保持不变，非 TUN 仍使用源文件。证据见[回归失败记录](../target/tun-case-check/run-84dfb518044e427d8f016bbdec2ffe64/output/red-result.json)、[完整测试输出](../target/tun-case-check/run-84dfb518044e427d8f016bbdec2ffe64/output/green-stdout.txt)和[Xray 配置校验结果](../target/tun-case-check/run-84dfb518044e427d8f016bbdec2ffe64/output/xray-cases-result.json)。校验脚本的进程等待及 PowerShell 5.1 结果导出问题修正后重跑通过；本次未重复设备移除或 GUI 验收。

### 核心多版本管理 · 2026-11-16

将覆盖式更新改为各核心页面内的“管理版本”。首次打开时获取官方发布列表，每页 30 条，可刷新或加载更多；启动应用时只扫描本地。列表标注稳定版、预览版及本地状态，本地版本置顶，同组按版本号降序排列。两个核心分别持久化所选版本，取消原通道选择。

核心及配套资源存入 `<核心>_core/versions/<版本号>/`。下载完成只增加该版本，不停止核心或自动切换；大小、SHA-256、归档路径及实际版本验证通过后，原子提交暂存目录。历史安装包只要求 EXE，随包 DLL 按白名单提取。已有目录不覆盖。切换先保存选择，再停止目标核心，保持停止，由用户手动启动；原 JSON、配置选择、启动工作目录和另一核心保持不变。删除需二次确认，当前版本禁止删除，目录穿越及父级/版本目录重解析点被拒绝。损坏标签元数据或缺少 EXE 的普通版本目录仍可列出、删除后重下。

自动检查：`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、90 项离线测试、2 项显式官方联网安装测试，以及 Debug / Release 构建通过。新增回归覆盖版本共存、明确选择与降级、下载不停止进程、保存失败保留原进程、当前版删除保护、退出后的迟到下载、缺文件目录恢复和路径边界。异步版本探针与下载解耦，以所选版本和文件指纹验证结果，避免下载其他版本时永久丢失当前版本文字。

Windows GUI 验收使用独立 package/lib/bin、实例锁和状态目录，`asInvoker`、模拟 DNS、空 TUN 清理，移除测试副本开机与 panic 恢复入口。生产 `ui.rs`、`runtime.rs`、`update.rs` 与副本一致。125% 缩放下实测 820×510 客户区浅色/深色及 660×440 深色最小客户区：列表标签、当前版本、按钮与分页可见；启动模拟 sing-box 后切换版本，进程停止且选择保存；Enter 可取消删除，确认删除只移除目标版本；重开恢复选择。通过 GUI 下载官方 sing-box `1.15.0-alpha.13`，显示进度，下载后与两个模拟版本共存，所选版本仍为 `1.1.0-beta.1`；实际加载第 2 页成功并保留本地版本。

界面记录见[浅色版本列表](../target/version-manager-gui/evidence/light.png)、[深色版本列表](../target/version-manager-gui/evidence/dark.png)、[最小窗口](../target/version-manager-gui/evidence/minimum.png)、[下载进度](../target/version-manager-gui/evidence/progress.png)和[下载完成](../target/version-manager-gui/evidence/downloaded.png)。未重复 ARM64 实机、完整 DPI 矩阵、真实 DNS/TUN 和整机重启验收。UI 采用现有 GPUI 控件；已检索网页响应式表格指南，因不适用于原生界面未采用。

### 提示范围与回归测试修复 · 2026-11-16

修复运行配置失败出现在“管理版本”的问题。后台分别保存运行错误、版本操作错误和设置错误；前台命令错误按核心及页面保存。外部编辑器、目录打开等异步结果使用发起时的范围，完成时即使已切页也不会串到当前页面。运行错误只在对应配置页显示，下载进度、版本切换结果和版本检测错误只在对应版本页显示；日志写入警告移回日志页。当时仍保留 DNS 全局提示，已在下一节的补充修复中移除。

原回归脚本错误地为每个场景重新启动可见桌面程序，造成 24 次启动；已移除该循环启动方式，旧入口转发至单进程脚本。新脚本只启动一个普通权限测试进程、创建一个 `show: false` / `focus: false` 的 GPUI 窗口，在同一实例中依次检查 24 个场景。每案调用实际页面渲染函数，检查运行错误、版本错误及成功消息、设置错误、命令错误和切页后返回的异步错误；同时通过 Windows API 确认窗口隐藏且未成为前台窗口。测试采用独立空目录和模拟 DNS，禁用托盘及正常桌面启动入口，不启动核心或发起下载；设置 30 秒超时，仅清理脚本自己持有的测试进程。

修复前的真实渲染记录已复现跨页错误；修复后的 24 项单进程检查全部通过，测试进程正常退出。新增后台回归确认运行操作与版本操作不会互相清除错误，91 项离线测试通过，2 项官方联网测试本轮保持忽略。`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、Debug / Release 构建及独立复核通过。此次验证覆盖提示内容的渲染范围，未重复视觉布局、DPI 或真实 DNS/TUN 验收。

证据见[单进程测试结果](../target/feedback-scope-check/single-process-result.json)、[隔离与源码记录](../target/feedback-scope-check/single-process-instrumentation.json)及 `target/feedback-scope-check/evidence/` 中的修复前记录。验收脚本为本机临时文件 `check-ladder-feedback-scope.py`（入口）和 `check-ladder-feedback-hidden.py`（实现）。

### 提示范围补充修复与可见验收 · 2026-11-16

用户通过断网发现 sing-box 与 Xray 版本页出现同一句网络错误。上一轮 24 项检查只注入快照，没有覆盖首次进入版本页自动请求列表的动作链。新增验收实际调用页面操作、Runtime 和 HTTP 客户端，并记录请求所属核心与端点：sing-box 下载失败后，已加载列表的 Xray 页面没有错误；首次打开尚未加载的 Xray 版本页时，Xray 自己的列表请求也失败，两种失败此前使用相同文案。现分别显示“sing-box v… 下载失败”和“xray 版本列表加载失败”，保留首次打开自动获取列表的行为。

同时移除根容器的全局提示区，将 DNS 恢复错误、重试入口和开机任务警告全部放入软件设置页。启动时存在设置或 DNS 故障则打开设置页，保证原因仍可见。版本操作期间，配置页只展示实际运行状态，继续禁用冲突操作；版本管理状态不再显示在配置页。

扩展的单隐藏进程矩阵覆盖两个核心、六个页面及 13 类反馈，共 156 项；修复前复现 20 项 DNS 跨页显示，修复后全部通过。断网动作链在一个隐藏进程中验证两条路径，旧文案基线失败，新版通过，均确认后台字段独立、返回源页面仍保留原错误。网络失败仅施加于测试进程的本地拒绝代理，不修改系统网络。测试记录见[矩阵修复前](../target/feedback-scope-matrix/red-result.json)、[矩阵修复后](../target/feedback-scope-matrix/single-process-result.json)、[请求链修复前](../target/network-flow-check/result-red.json)和[请求链修复后](../target/network-flow-check/result-green.json)。

本轮另外只打开一个可见的隔离验收窗口，实际点击下载与页面切换，核对 [sing-box 下载错误](../target/notices-visible-check/evidence/sing-box-download-error.png)、[Xray 无下载错误](../target/notices-visible-check/evidence/xray-no-download-error.png)、[Xray 自己的列表错误](../target/notices-visible-check/evidence/xray-own-list-error.png)、[配置页无版本错误](../target/notices-visible-check/evidence/config-no-version-error.png)、[设置页只显示 DNS 提示](../target/notices-visible-check/evidence/settings-dns-only.png)及[日志页无跨页横幅](../target/notices-visible-check/evidence/logs-no-banner.png)。测试版本元数据预置，DNS 故障为模拟值；UI 来自当前源码，失败经过实际网络请求路径，不安装或启动核心。窗口已关闭，测试进程无残留。

91 项离线测试、格式检查、Clippy、Debug / Release 构建及独立只读复核通过；两项官方联网安装测试保持忽略。本轮可见验收为 125% 缩放的默认窗口，没有重复完整 DPI、深色、真实 DNS/TUN 或 ARM64 实机矩阵。

### 统一可见滚动条 · 2026-11-16

配置列表、版本列表、两个核心页面外层、版本页面外层、软件设置、长错误信息与应用日志统一使用日志原有样式的滚动条。内容溢出时持续显示，支持滚轮、拖动滑块和点击轨道翻页；短内容不显示滑块，不增加提示文字。保留原列表 ScrollHandle、键盘定位及日志自动跟随逻辑。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings` 和 Debug 构建通过。使用 Computer Use 在一个可见的纯 UI 验收窗口中完成实际检查：浅色版本列表滚轮滚动、拖到最后一项、配置列表与日志滚动条显示、设置页和多个长错误的独立滚动、深色版本与设置、短列表无滚动条，以及版本页内外层滚动和轨道翻页。窗口采用测试数据与无副作用 stub，不编入真实 Runtime、DNS、TUN、恢复、核心进程或下载模块；验收后已关闭。证据见[浅色版本](../target/scrollbars-check/evidence/versions-light.png)、[拖到末尾](../target/scrollbars-check/evidence/versions-end.png)、[深色版本](../target/scrollbars-check/evidence/versions-dark.png)、[错误独立滚动](../target/scrollbars-check/evidence/error-end.png)和[内外层滚动](../target/scrollbars-check/evidence/nested-after.png)。本轮未完成最小窗口、完整 DPI 或键盘矩阵。

此前运行完整测试得到 86 项通过、5 项超时失败、2 项忽略，期间用户报告断网，并通过重新连接无线网络、重启 ladder-main 恢复。测试会启动同名模拟核心，ladder-main 按进程名识别核心；未查明断网的直接原因，不能据此断言修改了真实 DNS。完整测试未重跑，后续此类进程测试应放在独立 Windows 沙盒。两轮隐藏 UI 检查分别因未完成窗口尺寸初始化、窗口可见性或焦点断言失败，均不算验收通过；最终以本轮 Computer Use 可见检查为证据。

### 侧边栏精简与核心运行标识 · 2026-11-16

左上角只保留 Ladder，移除“核心启动器”和底部软件版本号。“应用日志”导航改为“日志”，移至侧栏底部、“软件设置”上方。sing-box 和 Xray 导航项仅在对应核心运行时，于右侧显示 8px 绿色圆点；复用现有运行快照、刷新链路及浅深色运行状态颜色。

`cargo fmt --check`、`git diff --check`、`cargo clippy --locked --offline --all-targets -- -D warnings` 和 `cargo build --locked --offline` 通过。独立只读复核确认导航与 Tab 顺序不变。本轮未启动应用或核心，未运行进程测试；实机画面及启停时圆点变化仍待验收。

### 版本管理页调整 · 2026-11-16

本轮取代此前“首次进入自动获取”和“本地版本优先”的交互：进入版本页只显示已有列表，手动点击“获取版本列表”才联网。当前版本置顶，其余使用现有数值版本比较降序排列，未新增发布时间字段或存储格式。“已下载版本”是独立于联网获取的筛选开关；“打开目录”定位对应核心根目录。标题下四按钮按约定横排，移除副标题；当前行右侧仅显示加粗文字，下载/切换用蓝字，删除及确认删除用红字。确认在原按钮内完成，点击其他位置或 Esc 取消。下载进度在该版本行中显示，重新获取列表时保留正在下载的行。浅色圆点改为更明亮的绿色，版本标签灰底略加深；浅深色的实心蓝、绿按钮均为白字。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、Debug 构建通过。三项精确过滤的纯测试覆盖 Release 解析、分页合并、当前置顶、已下载不额外优先，以及正式版/预览版数值排序；未运行启动进程的测试。独立复核发现切换后按钮消失会丢失键盘焦点，已在切换、确认删除及页面切换前移至持续存在的页面根焦点，复核通过。

使用 Computer Use 在一个纯 UI 隔离窗口内完成 125% 缩放、默认 820×510 客户区的实际验收：[首次进入只显示本地版本](../target/version-ui-check/evidence/initial-local.png)、[浅色完整列表](../target/version-ui-check/evidence/list-light.png)、[已下载筛选](../target/version-ui-check/evidence/local-filter.png)、[原位确认](../target/version-ui-check/evidence/confirm.png)、[点击空白取消](../target/version-ui-check/evidence/outside-cancel.png)、[Esc 取消](../target/version-ui-check/evidence/escape.png)、[确认删除后](../target/version-ui-check/evidence/deleted.png)、[切换后的 Tab 焦点](../target/version-ui-check/evidence/switch-focus.png)、[浅色行内进度](../target/version-ui-check/evidence/progress-light.png)、[深色行内进度](../target/version-ui-check/evidence/progress-dark.png)、[Xray 首次进入无自动获取](../target/version-ui-check/evidence/xray-local.png)、[深色 Xray 列表](../target/version-ui-check/evidence/list-dark.png)及[返回配置](../target/version-ui-check/evidence/return.png)。隔离副本使用生产渲染代码和纯版本比较函数，Runtime 为内存测试数据，不编入真实核心、网络、DNS、TUN、任务或恢复模块；外部目录操作为 no-op。窗口已关闭，未改动本机网络。未验证真实下载/文件删除/目录打开，未完成最小窗口或完整 DPI 矩阵。

### 下载进度同排显示 · 2026-11-16

将版本下载进度移到“稳定版 / 预览版”标签右侧，放在同一行；进度不换行、不额外增加版本行高度。sing-box、Xray 共用此布局。格式检查和 Debug 构建通过；在纯 UI 隔离窗口中，以 125% 缩放、660×440 最小客户区检查了[浅色](../target/version-ui-check/evidence/inline-progress-light-min.png)和[深色](../target/version-ui-check/evidence/inline-progress-dark-min.png)，进度完整显示，右侧按钮仍可见。未启动真实核心或网络操作，验收窗口已关闭。

### 标题旁刷新按钮 · 2026-11-16

获取列表入口改为版本标题右侧的 30px 蓝底白色刷新图标按钮，标题下只保留筛选、打开目录和返回配置。刷新图标用内置矢量路径绘制，不依赖外部资源或字体；悬停显示“获取版本列表”，获取期间禁用重复点击。两个核心共用布局。格式检查、Clippy 和 Debug 构建通过；纯 UI 隔离窗口核对了[浅色图标位置](../target/version-ui-check/evidence/refresh-button-light.png)、[点击获取与提示](../target/version-ui-check/evidence/refresh-button-loaded.png)、[深色](../target/version-ui-check/evidence/refresh-button-dark.png)及[Xray 布局](../target/version-ui-check/evidence/refresh-button-xray.png)。未启动真实核心或网络操作，验收窗口已关闭。

### 刷新按钮缩小与返回入口右对齐 · 2026-11-16

刷新按钮从 30px 缩至 24px，白色矢量图标从 18px 等比缩至 16px。“返回配置页面”在工具栏单独靠右，左侧筛选与目录按钮保持 8px 间距。格式检查、Debug 构建通过；在纯 UI 隔离窗口中完成[实际布局核对](../target/version-ui-check/evidence/small-refresh-right.png)，未启动真实核心或改动网络，窗口已关闭。

### 当前版本高亮铺满整行 · 2026-11-16

修复版本列表为滚动条保留独立右侧列而截断高亮的问题。版本行现在占满列表宽度，滚动条覆盖在右侧，行右内距保留文字和按钮的避让空间；不改其他滚动区。格式检查、Clippy 和 Debug 构建通过。纯 UI 隔离窗口实际核对[无滚动条](../target/version-ui-check/evidence/full-highlight-short.png)、[有滚动条](../target/version-ui-check/evidence/full-highlight-long.png)、[滚动后](../target/version-ui-check/evidence/full-highlight-scroll.png)及[深色](../target/version-ui-check/evidence/full-highlight-dark.png)，高亮均延伸至列表右边界，滚动条区域滚轮操作正常。窗口已关闭，未启动核心或改动网络。

### 默认已下载与分段筛选 · 2026-11-16

两个核心的版本列表默认选中“已下载”，原筛选按钮改为“已下载 / 全部”分段控件，沿用主题选择器的样式。选择直接设置筛选值，重复选择当前项不反转、不重置滚动；两个核心在本次运行中分别保留选择。刷新不改变当前筛选，切换筛选仍取消删除确认并回到列表顶部；Tab 顺序同步调整。

确认并补充文档：远端版本列表只在本次运行内存中保存，没有 TTL，不写入磁盘；切页及隐藏到托盘保留，完全退出后清除。手动刷新重新请求第一页，失败保留已有结果。本轮未修改缓存策略。

格式检查、`cargo clippy --locked --offline --all-targets -- -D warnings`、Debug 构建和两项精确过滤的纯测试（分页合并与版本排序）通过，独立增量复核未发现待修复问题。纯 UI 隔离窗口在 125% 缩放下验证了[默认已下载](../target/version-ui-check/evidence/segmented-default.png)、刷新保持筛选、[全部列表](../target/version-ui-check/evidence/segmented-all.png)、重复选择不反转、两个核心独立状态、[Tab/Shift+Tab 与 Enter/空格](../target/version-ui-check/evidence/segmented-keyboard.png)，以及 660×440 最小客户区的[浅色](../target/version-ui-check/evidence/segmented-min-light.png)和[深色](../target/version-ui-check/evidence/segmented-min-dark.png)布局。隔离窗口已关闭；未启动真实核心或改动网络，未重跑进程测试、真实联网下载、完整 DPI 矩阵及 Release 构建。

### 目录按钮右对齐与 GitHub 标签配色 · 2026-11-16

“打开目录”移到“返回配置页面”左侧，两按钮以 8px 间距成组靠右，筛选控件独立居左，Tab 顺序保持不变。版本标签使用独立颜色字段：稳定版浅/深色为 `#1a7f37` / `#3fb950`，预览版为 `#9a6700` / `#d29922`，对应 [GitHub Primer 的 success 与 attention 前景色](https://primer.style/product/getting-started/react/theme-reference/)。浅色标签背景改为 [Primer 的 bgColor-muted](https://primer.style/product/primitives/color/) `#f6f8fa`，深色背景保留 `#1c1e22`；文字对比度分别为 4.77、4.57、6.57、6.61。运行状态、侧栏圆点和 WARN 日志颜色保持原值。

`cargo fmt --check`、Clippy（全部 targets、warnings 作为错误）、Debug 构建及独立增量复核通过。纯 UI 隔离窗口在 125% 缩放、660×440 最小客户区下核对了[浅色](../target/version-ui-check/evidence/github-labels-light.png)和[深色](../target/version-ui-check/evidence/github-labels-dark.png)：目录/返回位置、稳定版与预览版颜色、筛选及滚动区布局正常。窗口已关闭；本轮未运行真实核心、进程测试或真实网络操作，未重跑完整 DPI 矩阵及 Release 构建。

### 返回按钮视觉高度校正 · 2026-11-16

核对源码及前轮截图：目录和返回按钮原本均为 30px；125% 缩放下两者外边界均为 37 个物理像素。蓝底边框与填充同色，比浅色描边显高。本轮仅将“返回配置页面”局部改为 28px，上下各内收 1px，保留蓝底、字号和容器居中对齐；其他按钮及点击行为不变。

格式检查、Clippy、纯 UI 隔离构建及独立增量复核通过；在 125% 缩放、660×440 最小客户区验证了[浅色](../target/version-ui-check/evidence/equal-button-height-light.png)与[深色](../target/version-ui-check/evidence/equal-button-height-dark.png)的视觉高度和文字对齐。隔离窗口已关闭。常规 Debug 构建最初因运行中的 `target/debug/ladder.exe` 被占用未能覆盖，用户退出后重试成功，调试版已更新；未运行真实核心、进程测试或修改网络。

### 侧栏运行圆点采用 Primer 绿色 · 2026-11-16

侧栏运行状态圆点采用与稳定版标签一致的 GitHub Primer success 前景色：浅色 `#1a7f37`、深色 `#3fb950`。仅修改 `running_dot` 色值，圆点大小、显示条件及其他状态颜色不变。独立复核确认，在侧栏普通、选中、悬停背景上的深浅色对比度均高于 3:1（最低 4.30:1）。格式检查、Clippy 和 Debug 构建通过，调试版已更新；本轮未重复 GUI 实测、未启动核心或修改网络。

### 精简版本提示与收紧分段筛选 · 2026-11-16

移除切换、下载、删除成功提示及空列表文案，同时删除已无用途的成功消息状态；错误信息、行内下载进度和刷新悬停提示继续保留。“已下载 / 全部”外层内边距从 2px 减为 1px，选项高度从 28px 减为 26px、最小宽度从 64px 减为 60px、左右内边距从 10px 减为 8px，字号保持 13px。整体高度从 34px 收至 30px，与右侧按钮协调。

格式检查、Clippy（全部 targets、warnings 作为错误）、Debug 构建及独立增量复核通过。纯 UI 隔离窗口在 125% 缩放、660×440 最小客户区核对了[浅色](../target/version-ui-check/evidence/compact-filter-light.png)、[深色](../target/version-ui-check/evidence/compact-filter-dark.png)、筛选切换，以及[已下载为空](../target/version-ui-check/evidence/empty-downloaded.png)和[全部为空](../target/version-ui-check/evidence/empty-all.png)：文字未裁切，空列表无文案。验收窗口已关闭；未启动真实核心或修改网络，未重跑完整 DPI 矩阵及 Release 构建。

### 提交前审查 · 2026-11-16

审查发现，已保存的当前版本目录被外部移走后，远端列表中的同版本仍被标为“当前使用”且缺少下载入口。仅将已安装且匹配选择的版本标为当前版本，使缺失目录的版本可重新下载。用户确认保留当前亮色：深浅主题统一使用稳定版/运行圆点 `#3fb950`、预览版 `#d29922`。这一决定取代前文分主题配色要求；前文色值及对比度为当时的历史验收记录。

规范与需求两项独立复审均无未解决问题。格式检查、Clippy、Debug 构建及三项精确过滤的纯逻辑测试（发布列表解析、分页合并、当前版本置顶与版本号排序）通过。纯 UI 隔离窗口验证了[缺失版本的下载入口](../target/version-ui-check/evidence/missing-current-download.png)、[点击后的模拟进度](../target/version-ui-check/evidence/missing-current-downloading.png)及[模拟下载完成后的当前版本](../target/version-ui-check/evidence/missing-current-restored.png)。窗口已关闭，未运行真实核心、联网下载或会修改本机网络的测试。

### 核心安装保留归档内容 · 2026-11-16

下载后的 ZIP 改为保留全部文件、子目录和空目录，不再按 EXE、DLL 和数据文件白名单筛选。按用户确认，只去掉统一的一层外包装目录，核心 EXE 仍直接位于版本目录下；配置示例、说明和许可证等随该版本一起保存。暂存版本探测与安装使用同一内容目录，已有版本和用户源配置保持不变。

1 GiB 解压限额改为累计全部文件。保留路径穿越与符号链接校验，拒绝 Windows 特殊路径、大小写重名和文件/目录冲突；允许普通空文件，核心 EXE 必须非空。归档中的 `.ladder-version.json` 若与应用版本记录冲突，安装报错，不覆盖归档文件。下载重试、超时与大小参数保持原值，并补充到 README。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo test --locked --offline update::tests:: -- --test-threads=1`（19 项通过，2 项官方联网安装验收忽略）、`cargo build --locked --offline` 和独立增量复核通过。回归覆盖 sing-box 包装目录、Xray 根目录、附带文件与空目录、同名文件、路径和元数据冲突及全文件大小限额。本轮未运行完整测试集、真实联网安装、GUI 或 Release 验收，未启动真实核心或修改本机网络。

## 后续开发注意点

- **配置编辑器权限**。行内“编辑”从 Ladder 调用系统关联程序；后续仍需复核管理员身份、文件关联与标准用户通过另一账户提升权限的行为。
- Windows 开机自启后续按实际问题和使用需求决定范围；需要改变当前产品行为时记录决定。
- 核心按版本独立存放；下载完成前的暂存目录不参与启动选择。后续若扩展安装事务，应继续保证已有版本不受中断影响。
- 阶段 6 才把打包内容作为交付契约固定下来。构建命令与环境依赖以 [README](../README.md) 为入口。
