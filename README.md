# Ladder

软件定位是 **sing-box** 和 **Xray** 的启动器, 功能足够简单, 配置完全由用户自己写 JSON 来决定, 几乎相当于运行裸核的方案, 只不过有 GUI 方便管理。

## 开发
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

### 编译

```powershell
# amd64
cargo build --release

# arm64 (vcvarsall.bat 路径需根据实际安装调整)
cmd /c "`"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvarsall.bat`" x64_arm64 && cargo build --release --target aarch64-pc-windows-msvc"
```

产出路径:

```text
target\release\ladder.exe                           # amd64
target\aarch64-pc-windows-msvc\release\ladder.exe   # arm64
```

### 检查与测试

```powershell
cargo fmt
cargo clippy
cargo test
```