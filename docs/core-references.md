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

通过后显示“配置合法”；失败信息去除 ANSI 控制序列并沿用敏感日志过滤，再显示于配置下方及对应核心日志。核心原始输出不写入应用日志文件。配置或核心程序变化、切换版本后清除旧结果；取消时清理尚未完成的验证状态。
