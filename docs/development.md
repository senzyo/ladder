# 开发与测试

## 模块

| 文件                                                           | 责任                                                   |
| -------------------------------------------------------------- | ------------------------------------------------------ |
| `core.rs`                                                      | 核心类型、显示名称及配置与版本根目录                   |
| `runtime.rs`                                                   | 设置、核心选择、工作线程、验证状态、核心升级及退出协调 |
| `managed_child.rs`                                             | Windows Job、命令行和进程树生命周期                    |
| `latency.rs`                                                   | 测速配置副本、回环代理和 HTTP 响应延迟                  |
| `update.rs`                                                    | 核心 Release 列表、资产选择、下载、解包、版本检测      |
| `github.rs`                                                    | HTTPS 加速链接前缀校验和 API/资产地址拼接              |
| `app_update.rs` / `app_updater.rs`                             | Ladder 下载校验、可取消更新助手、异步界面状态          |
| `ui.rs` / `proxy_input.rs` / `log_view.rs` / `scrollbar.rs`    | 页面、前缀编辑、日志显示、滚动条                       |
| `lib.rs`                                                       | 程序入口、单实例、托盘和更新退出流程                   |
| `dns.rs` / `scheduler.rs` / `emergency.rs` / `panic_helper.rs` | DNS 快照及正常/开机/panic 恢复                         |
| `tun_config.rs` / `tun_cleanup.rs`                             | TUN 运行副本和受保护的残留清理                         |
| `logging.rs` / `core_output.rs`                                | 有界日志、核心输出与终端控制序列清理                   |
| `i18n.rs` / `i18n/*.rs`                                       | 系统语言匹配、语言选择和中英俄词条                     |

## 国际化

设置页支持 `zh-Hans`、`en-US` 和 `ru-RU`。未手动选择语言时读取 Windows 用户首选显示语言；简体中文、英语和俄语映射到上述语言, 其余语言 (含繁体中文) 使用 `en-US`。手动选择成功保存到 `ladder-state.json` 后立即更新界面和托盘, 重启后保留选择。旧配置缺少 `language` 字段时继续匹配系统语言, 读取配置不会补写文件。

词条按界面、运行时、更新、系统操作和应用入口存放在 `src/i18n/`。新增词条需同时提供中英俄文本；动态消息使用命名参数, 三种语言的格式字符串都会参与编译检查。错误和操作提示在生成时翻译, 已产生的提示不会在切换语言后重新翻译。已记录的诊断日志、第三方核心原始输出及系统错误详情保留原文。

## 构建环境

Rust stable、Visual Studio C++ Build Tools (x64 与 ARM64) 、Windows SDK、LLVM/Clang、PowerShell 7。优先使用仓库锁文件；本地发布脚本自动定位 MSVC 环境。

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
```

Debug GPUI 从构建机的 Cargo 源码目录读取着色器, 复制到其他机器或沙盒必须使用 Release 构建。

## 不影响主机网络的验证

不要在运行 VPN/核心的主机直接执行完整测试或真实 Ladder。启动真实程序可能恢复 DNS、注册开机恢复任务或清理 TUN；部分旧测试创建 `sing-box.exe` / `xray.exe` 同名模拟进程, 可能被同时运行的 ladder-tray 识别。完整进程与系统集成测试放到独立 Windows 沙盒或 GitHub Actions 临时 runner。

以下集合使用纯解析、临时文件、模拟 DNS 或本机回环 HTTP, 不启动真实核心, 不更改主机 DNS/TUN。离线 DNS 恢复测试只在事务中创建临时 HKCU 键, 结束时自动回滚:

```powershell
cargo test --locked --lib -- --skip runtime::tests --skip scheduler::tests --skip update::tests::version_commands --skip update::tests::successful_version_commands --skip update::tests::version_process --skip update::tests::official
cargo test --locked --lib mihomo_config_extensions
cargo test --locked --lib validation_command
cargo test --locked --lib validation_failures_for_locked_configs
cargo test --locked --lib app_update_preparation_recovers_after_dns_failure_and_cancellation
cargo test --locked --lib runtime::tests::startup_
cargo test --locked --lib runtime::tests::language_
cargo test --locked --lib runtime::tests::github_proxy_
cargo test --locked --lib scheduler::tests::boot_leaves_an_active_session_alone_and_retries_after_exit -- --exact
```

上面列出的验证命令测试只创建唯一名称的 `ladder-validation-fixture.exe`。`validation_requests_complete_and_reject_duplicates` 和 `validation_results_are_discarded_after_inputs_change` 则通过真实请求入口启动模拟核心, 随完整测试在隔离环境执行。模拟核心每个测试进程只编译一次, 各用例复制到独立临时目录。

软件更新助手测试只运行临时无害程序, 验证替换、重启、回滚、取消和设置保留:

```powershell
./scripts/test-release.ps1 -Executable ./target/debug/ladder.exe
```

参数中的真实 Ladder 只被读取版本和打包, 不执行。完整测试留给隔离环境:

```powershell
cargo test --locked --lib
# 可选联网验收: 下载官方核心, 只执行版本命令
cargo test --locked official_stable_install -- --ignored
cargo test --locked official_preview_install -- --ignored
```
