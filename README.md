# Ladder

软件定位是 **sing-box** 和 **Xray** 的启动器, 功能足够简单, 配置完全由用户自己写 JSON 来决定, 几乎相当于运行裸核的方案, 只不过有 GUI 方便管理。

双击 `ladder.exe` 时，Windows 会在打开界面或启动核心前请求管理员权限。取消 UAC 即取消本次启动。从已提升的终端启动时会沿用管理员权限，提示方式取决于 Windows 的 UAC 策略。

如果 Windows 提示缺少 `VCRUNTIME140.dll`，请在运行 Ladder 的那台 Windows 上安装与程序架构匹配的 Visual C++ v14 Redistributable，再打开 Ladder。amd64 构建选择 x64。沙盒和虚拟机需要在其内部安装。[微软官方下载](https://learn.microsoft.com/zh-cn/cpp/windows/latest-supported-vc-redist)

### DNS 劫持与恢复

“软件设置”中的“劫持DNS到本地回环”首次安装默认关闭，升级沿用已保存的开关值。更早的设置文件缺少这个字段时，新增开关取关闭值，原有设置继续生效。

开启后，每次启动 sing-box 或 Xray 都会把物理网卡的 IPv4 DNS 设为 `127.0.0.1`、IPv6 DNS 设为 `::1`。开关从下一次启动核心起生效。所用配置需要能在本地回环地址提供 DNS 服务。

Ladder 按网卡和地址族记录首次劫持前的静态服务器或自动获取状态。任一核心停止或自行退出时，即使另一个核心仍在运行，也会恢复记录中的原值并刷新 DNS 缓存。劫持期间手动修改的 DNS 会被覆盖。

每次打开 Ladder 都会尝试恢复遗留记录，包括旧版保存在程序目录中的 `dns-recovery.json`。开机恢复程序和机器级快照位于 `%ProgramFiles%\Ladder DNS Recovery`。Ladder 会尝试用新的恢复动作替换同名任务 `Reset_DNS_On_Boot`。注册成功后，Windows 开机时由 SYSTEM 恢复 DNS，无需打开 Ladder。恢复成功后清除对应记录，失败时可在界面查看原因并重试。

如果已有机器级快照暂时无法访问，关闭 DNS 劫持仍可启动核心。修复目录权限或等占用结束后，可在界面点击“重试恢复 DNS”。若界面提示本次使用便携记录，异常退出后需再次打开 Ladder 才能恢复。

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
