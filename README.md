# Ladder

Windows 上的 sing-box、Xray 和 mihomo 轻量启动器。你维护核心配置，Ladder 负责版本管理、配置验证、进程启停和托盘操作。

## 开始使用

1. 从 [GitHub Releases](https://github.com/senzyo/ladder/releases) 下载与你的 Windows 架构匹配的便携包，解压到可写目录。
2. 打开 `ladder.exe`，按 Windows 提示授予管理员权限。
3. 选择核心，进入“管理版本”，下载并切换到所需版本。
4. 点击“打开配置目录”，放入自己的配置文件，返回列表点击“验证”。
5. 切换到配置后会立即启动核心；当前配置可用“启动核心”启动。

如果提示缺少 `VCRUNTIME140.dll`，安装与程序架构对应的 [Microsoft Visual C++ v14 Redistributable](https://learn.microsoft.com/zh-cn/cpp/windows/latest-supported-vc-redist)。amd64 选择 x64，arm64 选择 ARM64；虚拟机和沙盒需在其内部安装。

## 配置与运行

| 核心 | 配置目录 | 文件格式 | 验证命令 |
| --- | --- | --- | --- |
| sing-box | `configs/sing-box/` | JSON / JSONC 内容（`.json` 文件） | `sing-box check -c <配置>` |
| Xray | `configs/xray/` | JSON / JSONC 内容（`.json` 文件） | `xray run -test -c <配置>` |
| mihomo | `configs/mihomo/` | YAML / JSON（`.yaml`、`.yml`、`.json`） | `mihomo -t -f <配置>` |

配置页顶部“验证全部”检查该核心的全部配置；每行也可单独验证，包括当前配置。通过后显示绿色“配置合法”，失败时在该配置下方和对应核心日志中显示经过敏感信息过滤的错误。更改配置或核心版本后需重新验证。验证只运行核心的检查命令，不通过 Ladder 的启动、TUN 清理或 DNS 劫持流程；核心可能读取或下载配置引用的资源。

当前配置置顶。切换配置会先保存选择，再停止并启动目标核心；保存失败保留原选择和进程，启动失败保留新选择便于修复后重试。其他核心和外部启动的同名进程不受影响。编辑调用系统关联编辑器，必要时回退记事本；修改当前配置后可从托盘重启。删除配置和版本均需在原按钮处二次确认，当前使用的项不能删除。

关闭窗口会隐藏到托盘。左键点击托盘图标恢复窗口；右键菜单提供各核心的“重启核心”“停止核心”“切换配置”，以及“打开程序目录”和“退出并终止”。重启会先停止目标核心再启动当前配置。

## 核心版本与 GitHub 加速

每个核心在本次运行首次打开“管理版本”时获取一次 GitHub Release 列表；之后使用缓存，点击刷新可重取。“已下载”筛选离线可用。稳定版和预览版分别标注，当前版本置顶，其余按版本号排列。

版本保存在 `sing-box_core/<版本>/`、`xray_core/<版本>/`、`mihomo_core/<版本>/`。下载与程序架构一致的压缩包，验证大小、SHA-256、归档路径和可执行文件版本后安装，保留配套文件。mihomo amd64 使用兼容较旧 CPU 的 v1 构建。下载不停止正在运行的核心，也不自动切换；切换版本会停止该核心，由你手动重新启动。

版本页的“GitHub 加速前缀”由三个核心和 Ladder 自身更新共用，保存到设置中。留空直连；可填入官方文档推荐的 `https://gh-proxy.org/`，或其他 HTTPS 前缀。前缀同时用于 GitHub API 列表/发布信息请求及安装包下载，修改后从下一次请求生效。用法为在完整 GitHub URL 前拼接前缀，参见 [GH-Proxy 文档](https://gh-proxy.com/docs/github-accelerator)。

HTTP 请求沿用环境中的 `ALL_PROXY`、`HTTPS_PROXY`、`HTTP_PROXY` 和 `NO_PROXY`，支持对应的小写名称；不自动读取 Windows 系统代理。列表请求限时 30 秒，下载的连接及每次读取限时 30 秒，整体下载不设总时长。失败后可手动重试。

## 设置与软件更新

“设置”提供外观、启动时隐藏到托盘、随 Ladder 启动各核心、DNS 劫持和“关于”。主题可跟随系统或固定浅色/深色。

在“关于”点击“检查更新”。发现新版后下载，校验通过再点击“退出并更新”：Ladder 停止全部托管核心并恢复 DNS，更新助手等待程序退出后原子替换 EXE 并重新打开。停止或恢复失败会取消这次安装；替换失败保留或恢复旧程序。更新只替换程序文件，保留设置、配置、核心版本与 DNS 恢复记录。更新助手失败会在程序目录留下 `update-error.log`。

内置更新只跟随 `senzyo/ladder` 的正式发行版，拒绝降级、错误架构、损坏或不符合发布格式的包。尚未发布正式版时，会提示没有可用发行版。也可退出 Ladder 后手动覆盖 `ladder.exe` 完成升级。

## DNS 与 TUN

“劫持DNS到本地回环”默认关闭，从下一次启动核心起生效。开启后把物理网卡 IPv4 / IPv6 DNS 设为 `127.0.0.1` / `::1`；配置需在这些地址提供 DNS 服务。**不要和 mihomo 的 `strict-route`、sing-box 的 `strict_route`、Xray 的 `autoSystemWfpBlockLeak` 同时开启。**

Ladder 按网卡、地址族记录原 DNS（静态服务器或自动获取）。任一托管核心停止或自行退出时恢复记录，即使其他核心仍在运行；劫持期间的手动 DNS 修改会被原值覆盖。打开 Ladder、Windows 开机及程序 panic 后均有恢复路径，失败记录保留供重试。机器级记录和开机恢复程序位于 `%ProgramFiles%\Ladder DNS Recovery`，任务名为 `Reset_DNS_On_Boot`；必要时降级到程序目录的便携恢复记录。设置页可重试恢复。

启用 TUN 的配置使用同目录下的 `.ladder-tun-*` 运行副本，为接口生成随机 `tun-` 名称；源配置不改写。正常停止、启动失败或核心退出时删除副本，强制结束后的残留在下次启动清理。此文件名前缀保留给运行副本。

TUN 残留清理沿用 ladder-main 的范围：孤立或异常 WinTUN 设备，以及描述以 `tun-` 开头的网络历史记录，可能包含其他 VPN 的残留；保护正在托管的接口和系统现存接口。清理失败只记日志，不阻止核心启动。

## 日志与目录

日志页按 sing-box、Xray、mihomo、Ladder 分源，各保留本次运行最近 500 条。暂停冻结显示快照，后台继续收集；复制取自当前来源的显示内容。核心 stdout/stderr 只保存在内存，移除 ANSI 控制序列并过滤明确的密码、令牌、私钥等敏感标记。错误为红色，警告为黄色。

程序目录的 `app.log` 每次启动覆盖，最多 2 MiB；不写配置内容、核心原始输出或 panic 原文。开机 DNS 恢复日志独立位于机器级恢复目录。`ladder-state.json` 保存用户设置和选择，不能作为发布包的一部分。

## 开发与发布

- [开发与测试说明](docs/development-plan.md)：模块、验收范围与安全测试约束。
- [构建和发布](docs/releasing.md)：本地脚本、GitHub Actions、版本与产物约定。
- [官方验证命令依据](docs/core-references.md)。

正式发布前不维护历史开发版本的数据迁移；正式发布后的升级需保持用户数据和 DNS 恢复记录兼容。
