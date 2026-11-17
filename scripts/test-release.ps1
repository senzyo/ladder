#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$Version = [Diagnostics.FileVersionInfo]::GetVersionInfo($Executable).ProductVersion
$TestId = [Guid]::NewGuid().ToString('N')
$TestRoot = Join-Path ([IO.Path]::GetTempPath()) "ladder-release-test-$TestId"
[IO.Directory]::CreateDirectory($TestRoot) | Out-Null

function Assert-True {
    param([bool]$Value, [string]$Message)
    if (-not $Value) { throw $Message }
}

function Start-TestHelper {
    param([string]$Stage)
    $script = Join-Path $Stage 'apply-update.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'apply-update.ps1') -Destination $script
    $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $process = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $script)) -WindowStyle Hidden -PassThru
    return $process
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
    $fixtureCode = @'
fn main() {
    if let Some(signal) = std::env::args_os().nth(1) {
        let signal = std::path::PathBuf::from(signal);
        while !signal.is_file() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return;
    }
    let path = std::env::current_exe().unwrap().with_file_name("launched.txt");
    std::fs::write(path, "launched").unwrap();
}
'@
    [IO.File]::WriteAllText($fixtureSource, $fixtureCode)
    $fixture = Join-Path $TestRoot "ladder-release-fixture-$TestId.exe"
    & rustc --edition 2024 $fixtureSource -o $fixture
    if ($LASTEXITCODE) { throw 'Unable to compile harmless update fixture.' }
    foreach ($scenario in @('success', 'rollback', 'cancel', 'checksum-failure', 'file-in-use', 'cancel-after-ready')) {
        $success = $scenario -eq 'success'
        $root = Join-Path $TestRoot "$scenario update"
        $stage = Join-Path $root '.ladder-app-update-test123'
        [IO.Directory]::CreateDirectory($stage) | Out-Null
        $targetName = "ladder-release-target-$TestId.exe"
        $target = Join-Path $root $targetName
        Copy-Item -LiteralPath $fixture -Destination $target
        $oldFile = [IO.File]::Open($target, [IO.FileMode]::Append)
        try { $oldFile.WriteByte(42) } finally { $oldFile.Dispose() }
        $oldHash = (Get-FileHash -LiteralPath $target).Hash
        $source = Join-Path $stage 'ladder.exe'
        if ($scenario -ne 'rollback') { Copy-Item -LiteralPath $fixture -Destination $source } else { [IO.File]::WriteAllText($source, 'Invalid executable for rollback test') }
        $preserved = [ordered]@{
            'ladder-state.json' = 'preserve-this-state'
            'configs/mihomo/test.yaml' = "mode: rule`ntun:`n  enable: false`n"
            'configs/sing-box/test.json' = '{"outbounds":[{"type":"direct"}]}'
            'configs/xray/test.json' = '{"outbounds":[{"protocol":"freedom"}]}'
            'configs/mihomo/rules/local.yaml' = "payload:`n  - DOMAIN,example.test`n"
        }
        foreach ($entry in $preserved.GetEnumerator()) {
            $path = Join-Path $root $entry.Key
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
            [IO.File]::WriteAllText($path, $entry.Value)
        }
        $plan = @{ target = $targetName; parent = 2147483647; sha256 = (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() }
        if ($scenario -eq 'checksum-failure') { $plan.sha256 = '0' * 64 }
        $helper = $null
        $parentProcess = $null
        $lockedFile = $null
        try {
            if ($scenario -eq 'cancel-after-ready') {
                $exitSignal = Join-Path $root 'exit-parent'
                $parentProcess = Start-Process -FilePath $fixture -ArgumentList ('"{0}"' -f $exitSignal) -WindowStyle Hidden -PassThru
                $plan.parent = $parentProcess.Id
            }
            if ($scenario -eq 'file-in-use') {
                $lockedFile = [IO.File]::Open($target, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            }
            $plan | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stage 'update.json') -Encoding utf8
            if ($scenario -notin @('cancel', 'cancel-after-ready')) {
                [IO.File]::WriteAllText((Join-Path $stage 'commit'), 'commit')
            }
            $helper = Start-TestHelper $stage
            if ($scenario -eq 'cancel-after-ready') {
                $timer = [Diagnostics.Stopwatch]::StartNew()
                while (-not (Test-Path -LiteralPath (Join-Path $stage 'ready'))) {
                    if ($helper.HasExited -or $timer.ElapsedMilliseconds -gt 10000) { throw 'Update helper did not become ready.' }
                    Start-Sleep -Milliseconds 25
                }
                Assert-True (-not $helper.HasExited -and -not $parentProcess.HasExited) 'Update helper did not wait for its live parent.'
                Assert-True (-not (Test-Path -LiteralPath (Join-Path $stage 'commit'))) 'Cancellation test unexpectedly committed the update.'
                Assert-True ((Get-FileHash -LiteralPath $target).Hash -ceq $oldHash) 'Update replaced the program before cancellation.'
                $helper.Kill()
                Assert-True ($helper.WaitForExit(5000)) 'Cancelled update helper did not exit.'
                [IO.File]::WriteAllText($exitSignal, 'exit normally')
                Assert-True ($parentProcess.WaitForExit(5000)) 'Cancelled update parent did not exit normally.'
                Assert-True ($parentProcess.ExitCode -eq 0) 'Cancelled update parent failed during normal exit.'
            }
            if (-not $helper.WaitForExit(15000)) { throw "Update helper timed out: $scenario" }
            $result = $helper.ExitCode
        } finally {
            if ($null -ne $lockedFile) { $lockedFile.Dispose() }
            foreach ($process in @($helper, $parentProcess)) {
                if ($null -ne $process) {
                    if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
                    $process.Dispose()
                }
            }
        }
        foreach ($fixtureProcess in @(Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($target)) -ErrorAction SilentlyContinue)) {
            if ($fixtureProcess.Path -eq $target -and -not $fixtureProcess.WaitForExit(5000)) { throw 'Harmless restart fixture did not finish.' }
        }
        Assert-True (($result -eq 0) -eq $success) "Update helper returned an unexpected result: $scenario"
        foreach ($entry in $preserved.GetEnumerator()) {
            Assert-True ([IO.File]::ReadAllText((Join-Path $root $entry.Key)) -ceq $entry.Value) "Update modified user data: $scenario / $($entry.Key)"
        }
        $expectedHash = if ($success) { (Get-FileHash -LiteralPath $fixture).Hash } else { $oldHash }
        Assert-True ((Get-FileHash -LiteralPath $target).Hash -ceq $expectedHash) 'Update or rollback left the wrong executable.'
        Assert-True ((Test-Path -LiteralPath (Join-Path $root 'launched.txt')) -eq ($scenario -in @('success', 'rollback'))) "Unexpected restart behavior: $scenario"
        if ($success) { Assert-True (-not (Test-Path -LiteralPath $stage)) 'Successful update left its staging directory.' }
        else {
            Assert-True ((Test-Path -LiteralPath (Join-Path $root 'update-error.log')) -eq ($scenario -ne 'cancel-after-ready')) "Unexpected update diagnostic: $scenario"
        }
        Write-Host "PASS: update helper $scenario; settings and configuration preserved."
    }
    Write-Host 'PASS: package allowlist, deterministic ZIP, SHA-256, Windows PowerShell 5.1 update/restart/rollback/checksum/locked-file/ready-cancellation, settings and configuration preservation.'
} finally {
    $resolved = [IO.Path]::GetFullPath($TestRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notmatch '^ladder-release-test-[0-9a-f]{32}$') { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
