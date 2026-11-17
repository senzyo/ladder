<p align="center">
    <img src="icons/ladder.ico" width="100px" align="center" />
    <h2 align="center">Ladder</h2>
    <p align="center">
        一个 Windows 上的 mihomo、sing-box 和 Xray 轻量启动器。
    </p>
</p>

用户维护核心配置, Ladder 负责版本管理、配置验证、进程启停等操作。仅适用于自己搭建节点、并希望掌控配置全部细节的用户。

**注意, 不支持导入分享链接来简单地使用, 你必须写完整的配置文件。**

![运行截图](screenshots/main.png)

## 开始使用

1. 从 [GitHub Releases](https://github.com/senzyo/ladder/releases/latest) 下载与你的 Windows 架构匹配的便携包并解压。
2. 运行 `ladder.exe`, 按 Windows UAC 提示授予管理员权限。
3. 选择核心, 进入“管理核心版本”, 下载并切换到所需版本。
4. 点击“打开配置目录”, 放入自己的配置文件, 返回列表点击“验证”。
5. 如果配置合法, 可点击“启动核心”来运行。

如果提示缺少 `VCRUNTIME140.dll`, 安装与程序架构对应的 [Microsoft Visual C++ v14 Redistributable](https://learn.microsoft.com/zh-cn/cpp/windows/latest-supported-vc-redist)。amd64 选择 x64, arm64 选择 ARM64；虚拟机和沙盒需在其内部安装。

## 一些注意事项
### GitHub 加速

在软件“设置”中统一开启 GitHub 加速, 默认关闭。加速前缀预填 `https://axisnow.gh-proxy.org/`, 可修改后保存；关闭开关会保留前缀并恢复直连。

升级后保留原有自定义前缀, 空前缀补为默认地址；旧配置没有开关状态时, 需在设置中手动开启。

开启后, 三个核心的版本查询、下载, 以及 Ladder 自身检查更新和下载安装包都使用该前缀, 按 [GitHub 加速文档](https://gh-proxy.com/docs/github-accelerator) 在资源地址前拼接。关于页的仓库链接仍在浏览器打开原始主页；核心进程自身的网络请求由各自配置控制。

### 配置合法检查

| 核心     | 配置目录            | 文件格式                                | 验证命令                   |
| -------- | ------------------- | --------------------------------------- | -------------------------- |
| mihomo   | `configs/mihomo/`   | YAML / JSON (`.yaml`、`.yml`、`.json`)  | `mihomo -t -f <配置>`      |
| sing-box | `configs/sing-box/` | JSON / JSONC 内容 (`.json` 文件)        | `sing-box check -c <配置>` |
| Xray     | `configs/xray/`     | JSON / JSONC 内容 (`.json` 文件)        | `xray run -test -c <配置>` |

配置页顶部“验证全部”检查该核心的全部配置；每行也可单独验证, 包括当前配置。

通过后显示绿色“配置合法”, 仅表示所选核心的检查命令通过, 不保证所有引用资源可用或实际运行成功。

失败时在该配置下方和对应核心日志中显示核心错误原文, 不过滤敏感信息；显示时去除终端控制序列并保留行长限制。

更改配置或核心版本后需重新验证。

验证只运行核心的检查命令, 不通过 Ladder 的启动、TUN 清理或 DNS 劫持流程；核心可能读取或下载配置引用的资源。

资源检查沿用各核心的原生行为, Ladder 不额外预检引用文件。例如:
- mihomo `1.19.34` 的 `-t` 可接受缺失本地 rule provider 的配置, 运行时记录加载错误但继续运行；该规则集没有可用规则时不会匹配, 流量按后续规则处理, 是否符合预期取决于配置。
- sing-box `1.12` 起已移除旧 GeoIP/Geosite 配置, 应改用 rule-set；缺失本地规则集也会报错。
- Xray 引用的 `geoip.dat` / `geosite.dat` 缺失会导致检查或启动失败。

### DNS 与 TUN

“劫持 DNS 到本地回环”, 虽然被说是脏实现, 但我觉得还挺好用。

默认关闭, 从下一次启动核心起生效。开启后把物理网卡 IPv4 / IPv6 DNS 设为 `127.0.0.1` / `::1`；配置需在这些地址提供 DNS 服务。

**不要和 mihomo 的 `strict-route`、sing-box 的 `strict_route`、Xray 的 `autoSystemWfpBlockLeak` 同时开启。**

Ladder 按网卡、地址族记录原 DNS (静态服务器或自动获取) 。

任一托管核心停止或自行退出时恢复记录, 即使其他核心仍在运行；劫持期间的手动 DNS 修改会被原值覆盖。

打开 Ladder、Windows 开机及程序 panic 后均有恢复路径, 失败记录保留供重试。

机器级记录和开机自动恢复的任务计划程序位于 `%ProgramFiles%\Ladder DNS Recovery`, 任务计划名为 `Reset_DNS_On_Boot`；必要时降级到程序目录的便携恢复记录。

开机恢复使用独立的小型 Rust 程序, 随 Ladder 内嵌分发并在启动时更新, 不包含桌面界面。

设置页可重试恢复。

启用 TUN 的配置使用同目录下的 `.ladder-tun-*` 运行副本, 为接口生成随机 `tun-` 名称；源配置不改写。正常停止、启动失败或核心退出时删除副本, 强制结束后的残留在下次启动清理。此文件名前缀保留给运行副本。

TUN 残留清理沿用 ladder-tray 的范围: 孤立或异常 WinTUN 设备, 以及描述以 `tun-` 开头的网络历史记录, 可能包含其他 VPN 的残留；保护正在托管的接口和系统现存接口。清理失败只记日志, 不阻止核心启动。

## 开发与发布

- [开发与测试说明](https://github.com/senzyo/ladder/blob/main/docs/development-plan.md): 模块、验收范围与安全测试约束。
- [构建和发布](https://github.com/senzyo/ladder/blob/main/docs/releasing.md): 本地脚本、GitHub Actions、版本与产物约定。
- [官方验证命令依据](https://github.com/senzyo/ladder/blob/main/docs/core-references.md)。
