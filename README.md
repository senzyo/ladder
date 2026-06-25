<p align="center">
    <img src="https://sing-box.sagernet.org/assets/icon.svg" width="100px" align="center" />&nbsp;&nbsp;
    <img src="https://xtls.github.io/logo-light.svg" width="100px" align="center" />
    <h2 align="center">sing-box_with_xray</h2>
    <p align="center">
        一个极简 Windows 托盘程序, 使用 sing-box TUN 搭配 Xray
    </p>
</p>

## 简介

sing-box 负责 TUN、路由和大部分流量处理。直连流量由 sing-box 直接出站, 代理流量由 sing-box 转发到本地 socks 出站, 再交给 Xray 与 VPS 通信。

本项目已由 PowerShell 迁移到 Rust, 原脚本方案见 [powershell](https://github.com/senzyo/sing-box_with_xray/tree/powershell) 分支。

main 分支为新方案, 程序不提供软件主界面, 运行后常驻系统托盘, 单击托盘图标即可打开菜单。

## 功能

- **重启** — sing-box / xray / 全部
- **终止** — sing-box / xray / 全部
- **更新** — sing-box / xray / 全部 (通过 GitHub Releases API 自动检测新版本, 支持 CDN 代理和 SHA256 校验)
- **切换配置** — sing-box / xray (自动扫描 `configs/sing-box/` 和 `configs/xray/` 下的 `.json` 文件)

重启 sing-box 时, 程序会自动随机化 TUN 接口名, 避免残留网卡导致冲突。

## 目录结构

```text
sing-box_with_xray/
├── sing-box_with_xray.exe   # 托盘程序
├── core/
│   ├── sing-box.exe          # sing-box 核心
│   └── xray.exe              # xray 核心
├── configs/
│   ├── settings.toml         # 程序配置 (代理、日志、下载重试)
│   ├── sing-box.json         # 当前使用的 sing-box 配置
│   ├── xray.json             # 当前使用的 xray 配置
│   ├── sing-box/             # 可切换的 sing-box 配置
│   └── xray/                 # 可切换的 xray 配置
└── icons/
    ├── ladder.ico            # 应用图标
    ├── green_circle.ico      # 运行中
    ├── yellow_circle.ico     # 未运行
    └── red_circle.ico        # 未安装
```

## 配置

编辑 `configs/settings.toml` 可调整以下设置:

| 配置项                      | 默认值                  | 说明                                   |
| --------------------------- | ----------------------- | -------------------------------------- |
| `gh_proxy.enabled`          | `true`                  | 是否启用 GitHub CDN 代理               |
| `gh_proxy.url`              | `https://gh-proxy.com/` | GitHub CDN 代理地址前缀                |
| `log.level`                 | `debug`                 | 日志级别 (debug / info / warn / error) |
| `download.max_retries`      | `3`                     | 下载重试次数                           |
| `download.retry_delay_secs` | `2`                     | 重试间隔 (秒)                          |

修改后需重启程序生效。

## 使用

1. 从 [Releases](https://github.com/senzyo/sing-box_with_xray/releases/latest) 下载对应架构的压缩包 (amd64 或 arm64) 。
2. 解压后, 编辑 `configs/sing-box.json` 和 `configs/xray.json` 配置你的节点。
3. 双击 `sing-box_with_xray.exe` 运行, UAC 提示时选择允许。
4. 在系统托盘中单击图标使用菜单。
5. 点击 `更新核心` -> `更新 sing-box 和 xray` 来下载核心程序。
6. 点击 `重新启动` -> `重启 sing-box 和 xray` 来启动核心程序。

程序需要管理员权限, 因为 sing-box TUN 和 DNS 缓存清理需要提升权限。

## 开发

### 依赖

- Rust stable toolchain
- Visual Studio 生成工具 2022
  - MSBuild 工具
  - 使用 C++ 的桌面开发
    - 可选:
      - MSVC v143 - VS 2022 C++ x64/x86 生成工具(最新)
      - Windows 11 SDK
      - 用于 Windows 的 C++ CMake 工具
  - 单个组件
    - MSVC v143 - VS 2022 C++ ARM64/ARM64EC 生成工具(最新)
- LLVM/Clang

### 编译

```powershell
# amd64
cargo build --release

# arm64 (vcvarsall.bat 路径需根据实际安装调整)
cmd /c "`"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat`" x64_arm64 && cargo build --release --target aarch64-pc-windows-msvc"
```

产出路径:

```text
target\release\sing-box_with_xray.exe                           # amd64
target\aarch64-pc-windows-msvc\release\sing-box_with_xray.exe   # arm64
```

### CI/CD

推送到 `main` 分支时自动运行 clippy 和测试。

推送 `v*` 标签或手动触发 workflow 时, 自动构建双架构并发布 GitHub Release。
