# 构建和发布

## 本地构建

安装 Rust stable、Visual Studio C++ Build Tools、Windows SDK 和 LLVM/Clang。MSVC 需包含 x64 与 ARM64 组件；脚本使用 `vswhere` 自动定位, 不固定 Visual Studio 年份或安装盘符。

在仓库根目录用 PowerShell 7 执行:

```powershell
./scripts/release.ps1                        # 两种架构
./scripts/release.ps1 -Architecture amd64
./scripts/release.ps1 -Architecture arm64
```

构建读取 Cargo.toml 版本, 使用 `cargo build --release --locked --target ...`。构建脚本会先编译同架构的小型 DNS 恢复助手并嵌入主程序, 无需另行分发助手。已经构建时可加 `-SkipBuild` 仅打包；脚本仍验证 EXE 的产品版本和 PE 架构。

产物位于 `dist/`:

```text
ladder-amd64-v<版本>.zip
ladder-amd64-v<版本>.zip.sha256
ladder-arm64-v<版本>.zip
ladder-arm64-v<版本>.zip.sha256
```

ZIP 顶层只包含 `ladder.exe`、`README.md`、`LICENSE`。不打包核心、配置、状态、日志、恢复记录或环境文件。文件顺序和 ZIP 时间戳固定, 相同输入文件产生相同 ZIP；这不宣称不同工具链编译出的 EXE 按位一致。

`.zip.sha256` 供手动核对；内置更新器使用 GitHub Release API 提供的资产 `digest` 校验下载包。

## GitHub Actions

`.github/workflows/release.yml` 在临时 Windows runner 上执行以下作业:

| 触发                   | 行为                                                                |
| ---------------------- | ------------------------------------------------------------------- |
| main 分支推送、PR      | rustfmt、clippy、完整 Rust 测试、打包与更新助手测试                  |
| 手动 workflow_dispatch | 检查后构建两种架构, 上传工作流产物, 不创建 Release                  |
| 推送 `v*` 标签         | 校验标签与 Cargo.toml 版本一致, 构建、校验并发布两种 ZIP 及 SHA-256 |

发布作业才授予 `contents: write`, 使用工作流的 GitHub token, 不需要提交个人 token。外部 Actions 固定到核实过的 commit SHA。预发布版本创建 GitHub prerelease, 内置自动更新只接受正式发布。

## 发版步骤

1. 同步修改根目录和 `dns-recovery/` 的 Cargo.toml 版本并更新各自的 Cargo.lock, 完成代码审核和检查。
2. 在隔离环境验证真实核心/托盘/DNS恢复, 在两种架构检查启动。
3. 提交代码, 将对应提交推送到 GitHub。
4. 创建并推送与 Cargo 版本一致的标签, 格式为 `v<版本>`；Actions 自动发布。
5. 查看工作流结果和四个发布资产；发布可接受的更高正式版时, 再用前一个适用版本测试“检查更新→下载更新→重启以更新”。预发布不作为内置更新目标。

也支持本地发布 (需要已登录 `gh`) :

```powershell
./scripts/release.ps1 -Publish
```

本地发布要求干净工作区、两种架构重新构建, 且远端精确版本标签指向当前 HEAD。禁止 `-SkipBuild -Publish`。它创建新 Release, 不覆盖现有 Release。

同一标签应选择 Actions 或本地一种发布路径。使用本地发布时, 须先安排该标签的 Actions 发布不同时执行。

## 软件升级契约

- 正式包名称、仓库、顶层 EXE 名和两个架构后缀必须保持与 `app_update.rs` 一致。
- 先验证 GitHub 官方资产 URL、大小、SHA-256、ZIP 白名单和 PE 架构, 解压到程序目录的独立暂存目录。
- 更新前停止托管核心并成功恢复 DNS；启动助手失败不让运行管理进入永久关闭状态。
- 助手就绪后, 退出成功才写入最终提交标记。退出失败销毁未提交助手, 后续正常退出不会误安装旧任务。
- PowerShell 5.1 助手等待原 PID 结束, 再校验 EXE 哈希并使用原子替换。替换或新进程启动失败时恢复旧 EXE；设置与配置不在替换范围。
- 失败标记为程序目录 `update-error.log`, 下次启动在“关于”提示。成功清除该标记。
