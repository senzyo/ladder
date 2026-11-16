#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateSet('amd64', 'arm64', 'all')]
    [string]$Architecture = 'all',
    [switch]$SkipBuild,
    [switch]$Publish
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ProjectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$DistRoot = Join-Path $ProjectRoot 'dist'

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program 执行失败。" }
}

function Import-VcEnvironment {
    param([string]$TargetArchitecture)
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw '请安装 Visual Studio C++ 生成工具。' }
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $installation) { throw '找不到 MSVC C++ 生成工具。' }
    $vcvars = Join-Path $installation 'VC\Auxiliary\Build\vcvarsall.bat'
    $hostArchitecture = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $targetArchitecture = if ($TargetArchitecture -eq 'arm64') { 'arm64' } else { 'x64' }
    $pair = if ($hostArchitecture -eq $targetArchitecture) { $hostArchitecture } else { "${hostArchitecture}_$targetArchitecture" }
    # cmd 只用于运行 VS 官方环境脚本，不执行文件删除或移动。
    $environment = & $env:ComSpec /d /c "call `"$vcvars`" $pair >nul && set"
    if ($LASTEXITCODE -ne 0) { throw "无法加载 $pair 编译环境，请安装对应的 MSVC 组件。" }
    foreach ($line in $environment) {
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
        }
    }
}

function New-ReleaseArchive {
    param([string]$Arch, [string]$Version, [string]$Executable)
    if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "缺少 $Arch 构建产物。" }
    $versionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($Executable)
    if ($versionInfo.ProductVersion -ne $Version) { throw '构建产物版本与 Cargo.toml 不一致，请重新构建。' }
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Executable))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw '构建产物不是 Windows 可执行文件。' }
        $reader.BaseStream.Position = 0x3c
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550) { throw '构建产物缺少 PE 头。' }
        $machine = $reader.ReadUInt16()
        if (($Arch -eq 'amd64' -and $machine -ne 0x8664) -or ($Arch -eq 'arm64' -and $machine -ne 0xaa64)) {
            throw '构建产物架构与安装包名称不一致。'
        }
    } finally { $reader.Dispose() }
    $name = "ladder-$Arch-v$Version.zip"
    $archivePath = Join-Path $DistRoot $name
    $temporary = Join-Path $DistRoot ([IO.Path]::GetRandomFileName())
    $files = [ordered]@{
        'LICENSE' = (Join-Path $ProjectRoot 'LICENSE')
        'README.md' = (Join-Path $ProjectRoot 'README.md')
        'ladder.exe' = $Executable
    }
    try {
        $stream = [IO.File]::Create($temporary)
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($entry in $files.GetEnumerator()) {
                $item = $archive.CreateEntry($entry.Key, [IO.Compression.CompressionLevel]::Optimal)
                $item.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                $inputFile = [IO.File]::OpenRead($entry.Value)
                $outputFile = $item.Open()
                try { $inputFile.CopyTo($outputFile) } finally { $inputFile.Dispose(); $outputFile.Dispose() }
            }
        } finally { $archive.Dispose(); $stream.Dispose() }
        Move-Item -LiteralPath $temporary -Destination $archivePath -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$archivePath.sha256", "$hash  $name`n", [Text.Encoding]::ASCII)
    Write-Host "已打包 $name"
    return $archivePath
}

Push-Location $ProjectRoot
try {
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw '读取 Cargo 项目版本失败。' }
    $metadata = $metadataText | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'ladder').version
    if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw '不支持的发行版本号。' }
    $architectures = if ($Architecture -eq 'all') { @('amd64', 'arm64') } else { @($Architecture) }
    if ($Publish -and $Architecture -ne 'all') { throw '正式发布必须同时提供 amd64 和 arm64。' }
    if ($Publish -and $SkipBuild) { throw '发布必须重新构建，不能使用 -SkipBuild。' }
    if ($Publish) {
        $changes = & git status --porcelain
        if ($LASTEXITCODE -ne 0 -or $changes) { throw '发布需要干净的 Git 工作区。' }
        $commit = & git rev-parse HEAD
        if ($LASTEXITCODE -ne 0) { throw '读取提交失败。' }
        $refs = @(& git ls-remote --tags 'https://github.com/senzyo/ladder.git' "refs/tags/v$version" "refs/tags/v$version^{}")
        if ($LASTEXITCODE -ne 0) { throw '无法读取 GitHub 发行标签。' }
        if ($refs.Count -eq 0) { throw "请先将当前提交推送到 GitHub 并创建对应标签 v$version。" }
        $peeled = @($refs | Where-Object { $_.EndsWith('^{}') })
        $remoteCommit = if ($peeled.Count) { ($peeled[0] -split '\s+')[0] } else { ($refs[0] -split '\s+')[0] }
        if ($remoteCommit -cne $commit) { throw 'GitHub 发行标签没有指向当前提交，停止发布。' }
    }
    [IO.Directory]::CreateDirectory($DistRoot) | Out-Null
    $artifacts = @()
    foreach ($arch in $architectures) {
        $target = if ($arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
        if (-not $SkipBuild) {
            Import-VcEnvironment $arch
            Invoke-Checked rustup @('target', 'add', $target)
            Invoke-Checked cargo @('build', '--release', '--locked', '--target', $target)
        }
        $exe = Join-Path $metadata.target_directory "$target/release/ladder.exe"
        $archive = New-ReleaseArchive $arch $version $exe
        $artifacts += $archive, "$archive.sha256"
    }
    if ($Publish) {
        $arguments = @('release', 'create', "v$version", '--repo', 'senzyo/ladder', '--verify-tag', '--title', "v$version", '--generate-notes')
        if ($version.Contains('-')) { $arguments += '--prerelease' }
        Invoke-Checked gh ($arguments + $artifacts)
    }
} finally { Pop-Location }
