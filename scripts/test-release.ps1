#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$Version = [Diagnostics.FileVersionInfo]::GetVersionInfo($Executable).ProductVersion
$TestRoot = Join-Path ([IO.Path]::GetTempPath()) ('ladder-release-test-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($TestRoot) | Out-Null

function Assert-True {
    param([bool]$Value, [string]$Message)
    if (-not $Value) { throw $Message }
}

function Invoke-TestHelper {
    param([string]$Stage)
    $script = Join-Path $Stage 'apply-update.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'apply-update.ps1') -Destination $script
    $fixtureScript = [IO.File]::ReadAllText($script).Replace('    if ($Replaced -and', '    [IO.File]::WriteAllText((Join-Path $Root "fixture-error.txt"), $_.Exception.ToString())' + "`n" + '    if ($Replaced -and')
    [IO.File]::WriteAllText($script, $fixtureScript)
    $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $process = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $script)) -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(15000)) { $process.Kill(); throw 'Update helper timed out.' }
    return $process.ExitCode
}

try {
    $project = Join-Path $TestRoot 'package project'
    [IO.Directory]::CreateDirectory((Join-Path $project 'scripts')) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $project 'src')) | Out-Null
    $binary = Join-Path $project 'target/x86_64-pc-windows-msvc/release/ladder.exe'
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($binary)) | Out-Null
    Copy-Item -LiteralPath $Executable -Destination $binary
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'release.ps1') -Destination (Join-Path $project 'scripts/release.ps1')
    [IO.File]::WriteAllText((Join-Path $project 'Cargo.toml'), "[package]`nname = `"ladder`"`nversion = `"$Version`"`nedition = `"2024`"`n")
    [IO.File]::WriteAllText((Join-Path $project 'src/main.rs'), 'fn main() {}')
    [IO.File]::WriteAllText((Join-Path $project 'README.md'), 'Test documentation')
    [IO.File]::WriteAllText((Join-Path $project 'LICENSE'), 'Test license')
    [IO.File]::WriteAllText((Join-Path $project '.env'), 'must-not-be-packaged')
    [IO.File]::WriteAllText((Join-Path $project 'ladder-state.json'), 'must-not-be-packaged')
    & (Join-Path $project 'scripts/release.ps1') -Architecture amd64 -SkipBuild | Out-Null
    $zipPath = Join-Path $project "dist/ladder-amd64-v$Version.zip"
    $hash = (Get-FileHash -LiteralPath $zipPath).Hash
    $archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        Assert-True (($archive.Entries.FullName -join ',') -ceq 'LICENSE,README.md,ladder.exe') 'Package allowlist was violated.'
    } finally { $archive.Dispose() }
    $checksum = Get-Content -LiteralPath "$zipPath.sha256" -Raw
    Assert-True ($checksum.StartsWith($hash.ToLowerInvariant())) 'Checksum did not match the archive.'
    & (Join-Path $project 'scripts/release.ps1') -Architecture amd64 -SkipBuild | Out-Null
    Assert-True ((Get-FileHash -LiteralPath $zipPath).Hash -ceq $hash) 'Packaging was not reproducible for identical input files.'

    $fixtureSource = Join-Path $TestRoot 'fixture.rs'
    [IO.File]::WriteAllText($fixtureSource, 'fn main() { let path = std::env::current_exe().unwrap().with_file_name("launched.txt"); std::fs::write(path, "launched").unwrap(); }')
    $fixture = Join-Path $TestRoot 'fixture.exe'
    & rustc --edition 2024 $fixtureSource -o $fixture
    if ($LASTEXITCODE) { throw 'Unable to compile harmless update fixture.' }
    foreach ($success in @($true, $false)) {
        $root = Join-Path $TestRoot $(if ($success) { 'successful update' } else { 'failed update' })
        $stage = Join-Path $root '.ladder-app-update-test123'
        [IO.Directory]::CreateDirectory($stage) | Out-Null
        $target = Join-Path $root 'ladder-test.exe'
        Copy-Item -LiteralPath $fixture -Destination $target
        $oldFile = [IO.File]::Open($target, [IO.FileMode]::Append)
        try { $oldFile.WriteByte(42) } finally { $oldFile.Dispose() }
        $oldHash = (Get-FileHash -LiteralPath $target).Hash
        $source = Join-Path $stage 'ladder.exe'
        if ($success) { Copy-Item -LiteralPath $fixture -Destination $source } else { [IO.File]::WriteAllText($source, 'Invalid executable for rollback test') }
        [IO.File]::WriteAllText((Join-Path $root 'ladder-state.json'), 'preserve-this-state')
        $plan = @{ target = 'ladder-test.exe'; parent = 2147483647; sha256 = (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() }
        $plan | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stage 'update.json') -Encoding utf8
        $result = Invoke-TestHelper $stage
        foreach ($fixtureProcess in @(Get-Process -Name 'ladder-test' -ErrorAction SilentlyContinue)) {
            if ($fixtureProcess.Path -eq $target -and -not $fixtureProcess.WaitForExit(5000)) { throw 'Harmless restart fixture did not finish.' }
        }
        if (($result -eq 0) -ne $success) { Get-Content -LiteralPath (Join-Path $root 'fixture-error.txt') }
        Assert-True (($result -eq 0) -eq $success) 'Update helper returned an unexpected result.'
        Assert-True ((Get-Content -LiteralPath (Join-Path $root 'ladder-state.json') -Raw) -ceq 'preserve-this-state') 'Update modified user settings.'
        $expectedHash = if ($success) { (Get-FileHash -LiteralPath $fixture).Hash } else { $oldHash }
        Assert-True ((Get-FileHash -LiteralPath $target).Hash -ceq $expectedHash) 'Update or rollback left the wrong executable.'
        Assert-True (Test-Path -LiteralPath (Join-Path $root 'launched.txt')) 'Updated or restored executable did not restart.'
        if ($success) { Assert-True (-not (Test-Path -LiteralPath $stage)) 'Successful update left its staging directory.' }
        else { Assert-True (Test-Path -LiteralPath (Join-Path $root 'update-error.log')) 'Failed update did not leave a diagnostic.' }
    }
    Write-Host 'PASS: package allowlist, deterministic ZIP, SHA-256, Windows PowerShell 5.1 update/restart/rollback, settings preservation.'
} finally {
    $resolved = [IO.Path]::GetFullPath($TestRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notmatch '^ladder-release-test-[0-9a-f]{32}$') { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
