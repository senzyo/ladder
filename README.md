# Ladder

软件定位是 **sing-box** 和 **Xray** 的启动器, 功能足够简单, 配置完全由用户自己写 JSON 来决定, 几乎相当于运行裸核的方案, 只不过有 GUI 方便管理。

双击 `ladder.exe` 时，Windows 会在打开界面或启动核心前请求管理员权限。取消 UAC 即取消本次启动。从已提升的终端启动时会沿用管理员权限，提示方式取决于 Windows 的 UAC 策略。

如果 Windows 提示缺少 `VCRUNTIME140.dll`，请在运行 Ladder 的那台 Windows 上安装与程序架构匹配的 Visual C++ v14 Redistributable，再打开 Ladder。amd64 构建选择 x64。沙盒和虚拟机需要在其内部安装。[微软官方下载](https://learn.microsoft.com/zh-cn/cpp/windows/latest-supported-vc-redist)

### DNS 劫持与恢复

“软件设置”中的“劫持DNS到本地回环”首次安装默认关闭，开关保存在 `ladder-state.json`，下次打开时沿用。正式发布前不兼容历史开发版本的设置格式。

开启后，每次启动 sing-box 或 Xray 都会把物理网卡的 IPv4 DNS 设为 `127.0.0.1`、IPv6 DNS 设为 `::1`。开关从下一次启动核心起生效。所用配置需要能在本地回环地址提供 DNS 服务。

Ladder 按网卡和地址族记录首次劫持前的静态服务器或自动获取状态。任一核心停止或自行退出时，即使另一个核心仍在运行，也会恢复记录中的原值并刷新 DNS 缓存。劫持期间手动修改的 DNS 会被覆盖。

每次打开 Ladder 都会尝试恢复尚未完成的 DNS 记录。开机恢复程序和机器级快照位于 `%ProgramFiles%\Ladder DNS Recovery`。Ladder 会注册或更新开机任务 `Reset_DNS_On_Boot`。注册成功后，Windows 开机时由 SYSTEM 恢复 DNS，无需打开 Ladder。恢复成功后清除对应记录，失败时可在界面查看原因并重试。

如果已有机器级快照暂时无法访问，关闭 DNS 劫持仍可启动核心。修复目录权限或等占用结束后，可在界面点击“重试恢复 DNS”。机器目录不可用且确认没有机器级快照时，程序可降级为在自身目录保存 `dns-recovery.json`；界面会提示本次使用便携记录，异常退出后需再次打开 Ladder 恢复。机器目录恢复可用后，会接续处理便携记录，防止漏恢复或重复覆盖已经恢复的网卡。该机制是当前版本需要的故障恢复能力。

### TUN 接口与残留清理

每次启动或重启核心时，Ladder 为配置中的每个 TUN 入站生成独立的 `tun-` 随机接口名，分别写入 sing-box 的 `interface_name` 和 Xray 的 `settings.name`。Xray 的 TUN 识别兼容字段名与协议值的大小写写法。源 JSON 保持不变；带 TUN 的配置使用同目录下的 `.ladder-tun-*.json` 运行副本，正常停止、启动失败或核心自行退出后删除，强杀遗留的副本在下次打开 Ladder 时清理。此前缀保留给运行副本，不在配置列表中显示。无 TUN 时仍使用原文件。支持 JSON 注释和尾随逗号，Xray 还支持 `#` 行注释。

应用启动、TUN 核心启动前及核心停止后会检查残留。清理范围沿用 ladder-main：所有孤立或异常 WinTUN 设备，以及描述以 `tun-` 开头的 Windows 网络历史记录，可能包含其他 VPN 的残留。按名称识别并保护正在托管的接口，健康设备不移除；网络记录清理跳过系统现存接口，无法枚举接口时跳过网络记录清理。清理失败记入应用日志，不阻止核心启动。停止时先按原有规则恢复 DNS，再清理设备。

## 核心安装与更新

在 sing-box 或 Xray 页面选择“稳定版”或“预览版”，再点击“安装核心”或“检查更新”，从对应项目的官方 GitHub Release 获取更新。两个核心分别记住选择，默认稳定版。预览版允许安装官方预发行版本，也接受更新的稳定版；切换通道不会自动降级本地版本。下载的架构与 Ladder 构建一致（amd64 或 arm64）。

检查发布信息和下载安装包都使用 HTTP 库的环境代理设置，按 `ALL_PROXY`、`HTTPS_PROXY`、`HTTP_PROXY` 的顺序查找（也识别小写名称），并遵循 `NO_PROXY`。当前构建支持 HTTP/HTTPS 代理，未启用 SOCKS 代理支持。未配置有效环境代理时直连；不自动读取 Windows 系统代理。若启用了 TUN 或 VPN，实际连接仍受系统路由影响。

下载期间核心继续运行，界面显示进度并允许启停。下载完成后先验证大小、SHA-256、归档内容和暂存核心的版本，再停止目标核心、等待文件释放并替换。**替换后保持停止，由用户手动启动**，另一核心继续运行。网络、校验或文件占用失败时可点击“重试更新”。

更新只替换包内的核心 EXE、随附 DLL，以及 Xray 的 `geoip.dat`、`geosite.dat`，保留源 JSON、当前配置选择和其他自定义文件。旧文件在各自核心目录保留为 `原文件名.old`（例如 `sing-box_core/sing-box.exe.old`），直到下次实际安装更新时清理并轮换。仅检查版本、无需更新、下载或校验失败时不会清理 `.old`。

替换发生文件错误时回滚已改动的文件；如果占用使回滚也失败，错误中会给出 `.old` 备份位置。强杀或断电中断替换时，需要检查核心目录中的 `.old` 并手动恢复，不会在下次启动时自动恢复该安装事务。

手动放入或替换核心仍然可用，版本显示会在后台重新检测。应用不会在打开时自动联网检查或安装更新。退出 Ladder 会取消尚未应用的下载。

## 应用日志

在左侧导航点击“应用日志”，可在主窗口查看本次运行最近 500 条日志，与 sing-box、Xray 页面并列。支持暂停或继续刷新、复制当前显示内容、打开日志目录及键盘翻页；切换页面会保留暂停状态和滚动位置。暂停时日志记录继续，显示和复制内容保持不变。

`ERROR` 日志整行显示为红色，`WARN` 显示为黄色，两者均适配浅色和深色主题；`INFO` 使用普通正文颜色。日志不提供 JSON 语法或 ANSI 终端颜色高亮。

程序目录中的 `app.log` **每次启动覆盖**，不保留历史日志、不轮转。文件最多 2 MiB，达到上限后写入一次提示并停止文件记录，界面仍继续更新最近 500 条。日志文件无法创建或写入时会显示原因，核心管理和 DNS 恢复仍继续执行。

日志使用 Windows 本机时间，格式为 `YYYY-MM-DD HH:mm:ss.SSS`，随系统时区设置变化；界面不再显示 UTC 标签。日志记录应用启动退出、核心操作与异常退出、DNS 恢复、核心更新和设置保存结果。只记录可用于排障的摘要，不记录配置内容、密钥、环境变量、核心原始输出或 panic 原文。需要核心详细日志时，请在核心自身的配置中设置日志输出。

开机 DNS 恢复的日志单独写入 `%ProgramFiles%\Ladder DNS Recovery\app.log`，每次取得恢复锁并执行恢复时覆盖，不与桌面应用争用文件；此文件可手动打开查看。

## 开发

开发阶段、已确认的 GUI 方案与每轮验收要求见 [开发计划](docs/development-plan.md)。后续界面、运行管理和发布工作按该计划推进。

### 依赖

- Rust stable toolchain
- Visual Studio 生成工具 2026
  - MSBuild 工具
  - 单个组件
      - Windows 通用 C 运行时
      - C++ 生成工具核心功能
      - 适用于 x64/x86 的 MSVC 生成工具(最新版)
      - Windows 11 SDK
      - 适用于 ARM64/ARM64EC 的 MSVC 生成工具(最新版)
- LLVM/Clang

### 检查与测试

```powershell
cargo fmt
cargo clippy --locked
cargo test --locked
```

联网安装验收需显式运行，会在临时目录下载并安装两种官方核心，仅执行版本命令：

```powershell
cargo test --locked official_stable_install -- --ignored
cargo test --locked official_preview_install -- --ignored
```

```powershell
cargo build --locked
Start-Process .\target\debug\ladder.exe
```

调试版的 GPUI 会从构建机的 Cargo 源码目录加载着色器。复制到沙盒或其他电脑运行时，请使用下面的 Release 构建，着色器已嵌入程序。

### 编译

```powershell
# amd64
cargo build --release --locked

# arm64 (vcvarsall.bat 路径需根据实际安装调整)
cmd /c "`"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvarsall.bat`" x64_arm64 && cargo build --release --locked --target aarch64-pc-windows-msvc"
```

产出路径:

```text
target\release\ladder.exe                           # amd64
target\aarch64-pc-windows-msvc\release\ladder.exe   # arm64
```
