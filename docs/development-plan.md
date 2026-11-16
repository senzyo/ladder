# 开发与测试

更新于 2026-11-17。用户操作见 [README](../README.md)，发布约定见 [构建和发布](releasing.md)。历史开发过程保留在 Git 历史中，本文只描述当前实现和验收边界。

## 实现约定

- Windows 原生 Rust / GPUI，默认和最小客户区均为 900×600 逻辑像素。
- 页面为 mihomo、sing-box、Xray、日志、设置，默认打开 mihomo；左上角使用应用图标和较大的 Ladder 标题。保持简洁，不增加次要说明横幅。
- 配置由外部编辑器维护；Ladder 不格式化源文件。TUN 随机名称只写入同目录运行副本。
- 只终止由本次 Ladder 启动、通过 Windows Job 持有的进程，不按进程名杀进程。
- 核心选择、配置选择与软件设置落盘成功后再更新状态；异步结果属于发起操作的核心和页面。
- 验证结果绑定配置和所选核心程序，文件或版本改变后失效。错误经脱敏显示于配置下方及对应核心日志，不把原始输出写入 app.log。
- 下载先进入暂存目录，通过校验才安装。软件更新助手只有收到最终提交标记并确认原进程退出才替换 EXE；未提交助手由持有者取消。
- 保留机器级、便携降级、panic 助手和开机 DNS 恢复路径。这些是当前异常恢复能力，不属于可删除的历史兼容代码。
- 正式发布前不维护历史开发版本数据迁移；正式发布后需维护已发布数据格式的升级兼容。

## 模块

| 文件 | 责任 |
| --- | --- |
| `runtime.rs` | 设置、核心选择、工作线程、验证状态、核心升级及退出协调 |
| `managed_child.rs` | Windows Job、命令行和进程树生命周期 |
| `update.rs` | 核心 Release 列表、资产选择、下载、解包、版本检测 |
| `github.rs` | HTTPS 加速前缀校验和 API/资产地址拼接 |
| `app_update.rs` / `app_updater.rs` | Ladder 下载校验、可取消更新助手、异步界面状态 |
| `ui.rs` / `proxy_input.rs` / `log_view.rs` / `scrollbar.rs` | 页面、前缀编辑、日志显示、滚动条 |
| `lib.rs` | 程序入口、单实例、托盘和更新退出流程 |
| `dns.rs` / `scheduler.rs` / `emergency.rs` / `panic_helper.rs` | DNS 快照及正常/开机/panic 恢复 |
| `tun_config.rs` / `tun_cleanup.rs` | TUN 运行副本和受保护的残留清理 |
| `logging.rs` / `core_output.rs` | 有界日志、核心输出和敏感字段过滤 |

## 构建环境

Rust stable、Visual Studio C++ Build Tools（x64 与 ARM64）、Windows SDK、LLVM/Clang、PowerShell 7。优先使用仓库锁文件；本地发布脚本自动定位 MSVC 环境。

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
```

Debug GPUI 从构建机的 Cargo 源码目录读取着色器，复制到其他机器或沙盒必须使用 Release 构建。

## 不影响主机网络的验证

不要在运行 VPN/核心的主机直接执行完整测试或真实 Ladder。启动真实程序可能恢复 DNS、注册开机恢复任务或清理 TUN；部分旧测试创建 `sing-box.exe` / `xray.exe` 同名模拟进程，可能被同时运行的 ladder-main 识别。完整进程与系统集成测试放到独立 Windows 沙盒或 GitHub Actions 临时 runner。

以下集合使用纯解析、临时文件、模拟 DNS 或本机回环 HTTP，不启动真实核心，不更改主机 DNS/TUN：

```powershell
cargo test --locked --lib -- --skip runtime::tests --skip scheduler::tests --skip update::tests::version_commands --skip update::tests::successful_version_commands --skip update::tests::version_process --skip update::tests::official
cargo test --locked --lib mihomo_config_extensions
cargo test --locked --lib validation_command
cargo test --locked --lib validation_failures_for_locked_configs
```

验证命令测试只创建唯一名称的 `ladder-validation-fixture.exe`。软件更新助手测试只运行临时无害程序，验证替换、重启、回滚、取消和设置保留：

```powershell
./scripts/test-release.ps1 -Executable ./target/debug/ladder.exe
```

参数中的真实 Ladder 只被读取版本和打包，不执行。完整测试留给隔离环境：

```powershell
cargo test --locked --lib
# 可选联网验收：下载官方核心，只执行版本命令
cargo test --locked official_stable_install -- --ignored
cargo test --locked official_preview_install -- --ignored
```

## 交互验收

使用独立纯 UI harness：复制生产 UI 文件并记录哈希，Runtime、日志、更新均替换为内存 stub，不编入 DNS、TUN、恢复、真实核心、网络和文件安装模块。不要把 harness 当作真实核心端到端验证。

检查浅/深色主题，900×600 最小窗口，长配置名，单项/批量验证及错误滚动，三核心导航，前缀输入/选区/粘贴/保存，设置 DNS 说明换行和开关可见性，以及关于页的检查、下载、已就绪和失败状态。真实托盘、管理员启动、真实核心运行与联网下载应在沙盒完成。

本轮实际验收记录见 [2026-11-17 验收](verification-2026-11-17.md)。
