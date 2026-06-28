param(
    [switch]$Amd64Only,
    [switch]$Arm64Only,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

$ESC = [char]27
$Red    = "$ESC[91m"
$Green  = "$ESC[92m"
$Yellow = "$ESC[93m"
$Cyan   = "$ESC[96m"
$NC     = "$ESC[0m"

function Info  { Write-Host "${Cyan}[提示]${NC} $args" }
function Ok    { Write-Host "${Green}[成功]${NC} $args" }
function Warn  { Write-Host "${Yellow}[警告]${NC} $args" }
function Fail  { Write-Host "${Red}[错误]${NC} $args"; Write-Host ""; Write-Host "按任意键退出..."; $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown"); exit 1 }

# ── 参数校验 ──
function Show-Help {
    Write-Host ""
    Write-Host "用法: .\ci.ps1 [选项]"
    Write-Host "选项:"
    Write-Host "  -Amd64Only    仅构建 amd64 架构"
    Write-Host "  -Arm64Only    仅构建 arm64 架构"
    Write-Host "  -Help         显示此帮助信息"
    Write-Host ""
    exit 0
}

if ($Help) { Show-Help }

$known = @('-Amd64Only', '-Arm64Only', '-Help')
foreach ($arg in $args) {
    if ($arg -notin $known) {
        Write-Host "${Red}[错误]${NC} 未知参数: $arg"
        Show-Help
    }
}

if ($Amd64Only -and $Arm64Only) { Fail "-Amd64Only 和 -Arm64Only 不能同时使用" }

$ProjectRoot = $PSScriptRoot
$DistDir = Join-Path $ProjectRoot "dist"

# ── 获取版本号 ──
function Get-Version {
    $meta = cargo metadata --no-deps --format-version 1 2>$null
    if (-not $?) { Fail "cargo metadata 失败" }
    $meta | ConvertFrom-Json |
        Select-Object -ExpandProperty packages |
        Where-Object name -eq "sing-box_with_xray" |
        Select-Object -ExpandProperty version
}

# ── 定位 vcvarsall.bat ──
function Find-VcVarsAll {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { Fail "未找到 vswhere.exe, 请安装 Visual Studio 或 Build Tools" }

    $installPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if (-not $installPath) { Fail "未找到包含 C++ 生成工具的 Visual Studio 安装" }

    $vcvarsall = Join-Path $installPath "VC\Auxiliary\Build\vcvarsall.bat"
    if (-not (Test-Path $vcvarsall)) { Fail "未找到 vcvarsall.bat: $vcvarsall" }

    $edition = Split-Path (Split-Path (Split-Path $vcvarsall)) -Leaf
    Info "vcvarsall ($edition): $vcvarsall"
    return $vcvarsall
}

# ── 打包单个架构 ──
function Package-Arch {
    param(
        [string]$Arch,
        [string]$TargetDir,
        [string]$Version
    )

    $exe = Join-Path $TargetDir "sing-box_with_xray.exe"
    if (-not (Test-Path $exe)) { Fail "$Arch 构建产物不存在: $exe" }

    $name = "sing-box_with_xray-$Arch-v$Version"
    $outDir = Join-Path $DistDir $name
    $zipPath = Join-Path $DistDir "$name.zip"

    if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force }

    New-Item -ItemType Directory $outDir | Out-Null
    Copy-Item $exe $outDir\
    Copy-Item (Join-Path $TargetDir "settings.toml") $outDir\
    Copy-Item -Recurse (Join-Path $TargetDir "configs") $outDir\
    Copy-Item -Recurse (Join-Path $TargetDir "icons") $outDir\
    Copy-Item (Join-Path $TargetDir "README.md") $outDir\
    Copy-Item (Join-Path $TargetDir "LICENSE") $outDir\

    Compress-Archive $outDir $zipPath
    Remove-Item $outDir -Recurse -Force

    $size = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
    Ok "$Arch 打包完成 -> dist/$name.zip ($size MB)"
}

# ═══════════════════════════════════════════════
# 主流程
# ═══════════════════════════════════════════════

$version = Get-Version
Info "sing-box_with_xray v$version"

# ── 清理 dist/ ──
if (Test-Path $DistDir) {
    Remove-Item $DistDir -Recurse -Force
}
New-Item -ItemType Directory $DistDir | Out-Null

# ── Build amd64 ──
if (-not $Arm64Only) {
    Info "构建 amd64..."
    Push-Location $ProjectRoot
    try {
        cargo build --release
        if (-not $?) { Fail "amd64 构建失败" }
    } finally {
        Pop-Location
    }
    Ok "amd64 构建完成"
}

# ── Build arm64 ──
if (-not $Amd64Only) {
    Info "构建 arm64..."
    $vcvarsall = Find-VcVarsAll
    Push-Location $ProjectRoot
    try {
        cmd /c "`"$vcvarsall`" x64_arm64 && cargo build --release --target aarch64-pc-windows-msvc"
        if (-not $?) { Fail "arm64 构建失败" }
    } finally {
        Pop-Location
    }
    Ok "arm64 构建完成"
}

# ── 打包 ──
if (-not $Arm64Only) {
    Info "打包 amd64..."
    Package-Arch -Arch "amd64" -TargetDir (Join-Path $ProjectRoot "target\release") -Version $version
}

if (-not $Amd64Only) {
    Info "打包 arm64..."
    Package-Arch -Arch "arm64" -TargetDir (Join-Path $ProjectRoot "target\aarch64-pc-windows-msvc\release") -Version $version
}

Write-Host ""
Ok "完成！产出目录: dist/"
Write-Host ""
Write-Host "按任意键退出..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
