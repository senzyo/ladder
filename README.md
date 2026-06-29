<p align="center">
    <img src="icons/ladder.ico" width="100px" align="center" />
    <h2 align="center">sing-box_with_xray</h2>
    <p align="center">
        一个极简的、管理 sing-box 和 Xray 核心的 Windows 托盘程序。
    </p>
</p>

## 简介

原方案用 PowerShell 脚本管理 sing-box 和 Xray, 现已归档到 [powershell](https://github.com/senzyo/sing-box_with_xray/tree/powershell) 分支。

main 分支是由 Rust 构建的新方案, 程序运行后常驻系统托盘, 单击程序图标即可打开菜单进行管理。

`configs` 目录下 `with_*.json` 的示例配置是用于双核心搭配的, 比如 `configs\sing-box\with_xray_ipv4_only.json` 和 `configs\xray\with_sing-box_[xhttp+reality].json` 这一组的分工如下:

```mermaid
flowchart LR
    A(用户软件) --> B(sing-box)
    B -->|直连流量| C(目标网站)
    B -->|需代理流量| D(本地 SOCKS)
    D --> E(Xray)
    E --> F(VPS)
    F --> C
```

当然, 你可以根据自身需求仅运行其中一个核心, 比如使用 `configs\xray\olny_xray_上行[xhttp+reality]下行[xhttp+tls+cdn].json`, 这取决于你自己怎样修改配置文件。

![运行截图](screenshots/main.png)

## 目录结构

```
sing-box_with_xray
├── configs
│   ├── sing-box                        # 可切换的 sing-box 配置
│   │   ├── with_xray_ipv4_only.json
│   │   └── with_xray_prefer_ipv4.json
│   ├── sing-box.json                   # 当前使用的 sing-box 配置
│   ├── xray                            # 可切换的 xray 配置
│   │   ├── with_sing-box_[xhttp+reality].json
│   │   └── olny_xray_上下行分离[xhttp+tls+cdn].json
│   └── xray.json                       # 当前使用的 xray 配置
├── icons
│   ├── green_circle.ico                # 核心程序运行中的应用图标
│   ├── ladder.ico                      # 托盘和通知的应用图标
│   ├── red_circle.ico                  # 核心程序未安装的应用图标
│   └── yellow_circle.ico               # 核心程序未运行的应用图标
├── settings.toml                       # 主程序配置
├── sing-box_core                       # sing-box 工作目录
│   ├── cache.db
│   ├── libcronet.dll
│   └── sing-box.exe
├── sing-box_with_xray.exe              # 主程序
└── xray_core                           # xray 工作目录
    ├── geoip.dat
    ├── geosite.dat
    ├── wintun.dll
    └── xray.exe
```

## 配置

编辑 `settings.toml` 可调整以下设置:

| 配置项                      | 默认值                  | 说明                     |
| --------------------------- | ----------------------- | ------------------------ |
| `gh_proxy.enabled`          | `true`                  | 是否启用 GitHub CDN 代理 |
| `gh_proxy.url`              | `https://gh-proxy.com/` | GitHub CDN 代理地址前缀  |
| `log.level`                 | `debug`                 | 日志级别                 |
| `download.max_retries`      | `3`                     | 下载重试次数             |
| `download.retry_delay_secs` | `2`                     | 重试间隔 (秒)            |

修改后需重启程序生效。

## 首次使用

1. 从 [Releases](https://github.com/senzyo/sing-box_with_xray/releases/latest) 下载对应架构的压缩包 (amd64 或 arm64) 。
2. 解压后, 编辑 `configs/sing-box/*.json` 和 `configs/xray/*.json` 配置你的节点。
3. 不要直接编辑 `configs/sing-box.json` 和 `configs/xray.json`, 防止切换配置时被覆盖。
4. 双击 `sing-box_with_xray.exe` 运行, UAC 提示时选择允许。
5. 在系统托盘中单击图标打开使用菜单。
6. 点击 `更新核心` -> `更新 XX` 来下载核心程序。
7. 点击 `切换 XX 配置` 来切换配置, 切换后对应的核心会被运行。

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

推送到 `main` 分支时自动运行 `cargo clippy` 和 `cargo test`。

推送 `v*` 标签或手动触发 workflow 时, 自动构建双架构并发布 GitHub Release。
