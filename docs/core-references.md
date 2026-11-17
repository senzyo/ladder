# 核心命令与上游约定

核对日期：2026-11-17。Ladder 将路径作为单独的带引号参数传递，不改写用户配置。

| 核心 | 运行 | 验证 | 版本 |
| --- | --- | --- | --- |
| sing-box | `sing-box run -D <核心版本目录> -c <配置>` | `sing-box check -D <核心版本目录> -c <配置>` | `sing-box version` |
| Xray | `xray run -c <配置>` | `xray run -test -c <配置>` | `xray version` |
| mihomo | `mihomo -d <配置目录> -f <配置>` | `mihomo -t -d <配置目录> -f <配置>` | `mihomo -v` |

- [sing-box 配置文档](https://sing-box.sagernet.org/configuration/)提供 `check` 命令；Ladder 的 sing-box 配置保持 JSON 格式。
- [Xray 命令参数](https://xtls.github.io/document/command.html)说明 `-test` 只检查配置，不启动服务。它是 `run` 的选项，不是 `test` 子命令。Ladder 的 Xray 配置保持 JSON；资源位置通过 `XRAY_LOCATION_ASSET` 指向核心版本目录。
- [mihomo 命令入口](https://github.com/MetaCubeX/mihomo/blob/Meta/main.go)定义 `-d`、`-f`、`-t`、`-v`。`-t` 执行配置解析后返回，不进入运行服务的分支。验证仍可能读取或下载配置引用的资源，不能等同于完全离线验证。
- mihomo 的配置列表接收 `.yaml`、`.yml` 和 `.json`。`-d` 使用配置目录，规则、provider 和缓存资源留在配置旁，不随核心版本切换搬迁。
- [mihomo TUN 文档](https://wiki.metacubex.one/config/inbound/tun/)说明 `tun.device` 是接口名称。启用 TUN 时，Ladder 在同目录 YAML 运行副本中随机化该字段；源文件保持不变。运行副本展开 YAML 合并键，保留其语义。

## mihomo 发布资产

[官方构建工作流](https://github.com/MetaCubeX/mihomo/blob/Meta/.github/workflows/build.yml)和[正式发布 API](https://api.github.com/repos/MetaCubeX/mihomo/releases/latest)确认：

- amd64 优先选择 `mihomo-windows-amd64-v1-v<版本>.zip`，缺失时使用旧名称 `mihomo-windows-amd64-compatible-v<版本>.zip`。两者采用 amd64 v1 基线；不默认选择要求较新 CPU 指令的 v3 包。
- arm64 使用 `mihomo-windows-arm64-v<版本>.zip`。
- ZIP 名称带版本号，内部 EXE 不带版本号，例如 `mihomo-windows-amd64-v1.exe`。安装时仅将核心程序归一命名为 `mihomo.exe`，保留其他归档文件。
- [滚动预览发布](https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/Prerelease-Alpha)使用固定标签 `Prerelease-Alpha`，资产名和版本输出包含 `alpha-<提交哈希>`。Ladder 按该哈希保存独立本地版本，标记为预览版。下载前重新获取发布详情；若滚动发布已变化，拒绝安装不匹配资产并提示刷新列表。

所有核心下载均校验官方资产地址、声明大小、SHA-256 和暂存程序的版本。GitHub 加速前缀只包装请求地址，不改变官方地址校验依据。

## 验证结果

验证使用所选核心版本，单项或当前核心的全部配置均在后台串行检查。每项限时 30 秒、输出上限 64 KiB，所有验证进程归入独立 Windows Job。它不经过 Ladder 的核心启动、DNS 劫持或 TUN 清理流程。

通过后显示“配置合法”，仅表示所选核心的检查命令通过，不保证所有引用资源可用或实际运行成功。Ladder 沿用原生检查结果，不增加本地资源预检。失败信息去除 ANSI 控制序列并沿用敏感日志过滤，再显示于配置下方及对应核心日志。核心原始输出不写入应用日志文件。配置或核心程序变化、切换版本后清除旧结果；取消时清理尚未完成的验证状态。

## 资源加载与运行诊断

2026-11-17 的 AMD64 Windows 沙盒实测边界如下：

| 核心版本 | 配置引用 | 原生行为 |
| --- | --- | --- |
| mihomo `1.19.34` | 缺失本地 rule provider | `-t` 仍通过；实际运行记录加载 `ERROR`，进程继续运行。影响取决于可用规则及后续规则，不能直接判定整份配置不可用。 |
| Xray `26.11.9` | `geosite:cn` 或 `geoip:cn`，相应 `.dat` 文件缺失 | `run -test` 和 `run` 均退出 `23`，错误包含对应文件名。 |
| sing-box `1.14.4` | 本地 source `.json` 或 binary `.srs` 规则集缺失 | `check` 和 `run` 均退出 `1`，错误包含对应文件名。 |
| sing-box `1.14.4` | 旧 `geosite` / `geoip` 规则 | `check` 和 `run` 均退出 `1`，明确提示这些功能已在 `1.12.0` 移除；不是缺少旧数据库。 |

Xray 的[规则解析](https://github.com/XTLS/Xray-core/blob/v26.11.9/common/geodata/rule_parser.go)将 `geosite:` / `geoip:` 映射到对应 `.dat` 并检查文件，[命令入口](https://github.com/XTLS/Xray-core/blob/v26.11.9/main/run.go)规定配置初始化失败时退出 `23`。sing-box 的[弃用说明](https://sing-box.sagernet.org/deprecated/#geoip)记录 GeoIP/Geosite 在 `1.8` 弃用、`1.12` 移除，应迁移到 rule-set；[本地规则集实现](https://github.com/SagerNet/sing-box/blob/v1.14.4/route/rule/rule_set_local.go)在初始化时读取 source 或 binary 文件并返回读取错误。

mihomo `1.19.34` 的 [`-t` 分支](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/main.go#L175-L190)完成配置解析后返回，不进入运行时的 provider 初始化；解析仍会[检查引用的 provider 是否已声明](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/config/config.go#L1121-L1125)。运行时 [`loadProvider`](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/hub/executor/executor.go#L318-L352)仅记录初始化错误，[随后仍进入运行状态](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/hub/executor/executor.go#L115-L119)。因此加载成功不是这一启动路径的硬条件，但 `-t` 通过也不能说明配置不需要该 provider。

源码中 provider [先创建空规则策略，也支持可选 payload 回退](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/rules/provider/provider.go#L125-L144)；[空 domain 规则集不匹配](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/rules/provider/domain_strategy.go#L26-L28)，[`RULE-SET` 委托该 provider 匹配](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/rules/provider/rule_set.go#L23-L35)，[未命中则继续后续规则](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/tunnel/tunnel.go#L663-L690)。

随后用 mihomo `1.19.34` 完成真实 HTTP 请求对照：两份隔离配置均为 `mode: rule`，file/domain provider 没有配置内的 `payload` 备用规则，规则依次为 `RULE-SET,local,REJECT`、`MATCH,DIRECT`。有效文件仅匹配 `localhost`；另一份缺失同名文件。请求直接发送到核心的环回代理端口，目标为自建环回 HTTP 服务；配置内将 `localhost` 映射到 `127.0.0.1`，不修改系统 hosts，不启用 DNS/TUN，也不请求外网。

| 对照 | 原生检查与进程 | 实际请求与日志 |
| --- | --- | --- |
| provider 文件有效 | `-t` 退出 `0`；请求后核心仍运行 | `RuleSet(local) using REJECT`，客户端收到 HTTP `502`，目标服务器收到 `0` 次请求。 |
| provider 文件缺失 | `-t` 退出 `0`；记录 provider 加载 `ERROR`，请求后核心仍运行 | `Match using DIRECT`，客户端收到 HTTP `200` 和服务器生成的唯一标记，目标服务器收到 `1` 次请求。 |

证据位于 `target/handoff-20261117/output/run-30.log` 及同目录的 `mihomo-fallback/20261117-013649-4d8f2b/`；后者保存二进制版本/哈希、两份配置、原生检查与运行日志、逐项请求及服务器计数。结果证明此 provider 不是进程启动的硬条件，但缺失时原本应被拒绝的请求改走了后续 `DIRECT`，不能视为预期分流完整。其他配置若有可用缓存、备用 payload 或不同后续规则，影响可能不同；本例也不代表外网连通性或 Ladder 界面验收。

Xray 与 sing-box 的独立资源验收共 18 项全部通过，包含无需资源的基线、有效本地规则集及上述负例；证据位于本机忽略目录 `target/handoff-20261117/output/run-18.log` 和同目录 `resource-evidence/`。这些原生进程使用无入站、无 DNS/TUN 的隔离配置；正例运行只观察短时存活，不能据此宣称网络功能或 Ladder 界面已通过验收。mihomo 的检查与运行边界见[本轮验收记录](verification-2026-11-17.md)。

从 `4fd63bb` 起，三核心运行期间的最近一条已识别错误会在脱敏后进入主界面，也保留在对应核心日志。核心自行退出时先排空 stdout/stderr，再保留最后的错误，包括启动后立即退出的情况；停止或开始新一轮运行时清除主界面诊断。每条输出限制为 8 KiB，敏感行隐藏后仍保留错误级别。此诊断与操作失败提示独立，不把原生错误全文写入 `app.log`；应用日志只记录概括原因。
