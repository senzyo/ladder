# This script is embedded in Ladder and also supports Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$Stage = [IO.Path]::GetFullPath($PSScriptRoot)
$Root = [IO.Path]::GetDirectoryName($Stage)
$Backup = Join-Path $Stage 'previous.exe'
$Target = $null
$Replaced = $false

try {
    if ([IO.Path]::GetFileName($Stage) -notmatch '^\.ladder-app-update-[A-Za-z0-9]+$') {
        throw 'Invalid update directory.'
    }
    if ((Get-Item -LiteralPath $Stage).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Update directory cannot be a link.'
    }
    $plan = Get-Content -LiteralPath (Join-Path $Stage 'update.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([IO.Path]::GetFileName($plan.target) -cne $plan.target -or $plan.target -notmatch '\.exe$' -or
        $plan.sha256 -notmatch '^[0-9a-f]{64}$' -or $plan.parent -le 0) {
        throw 'Invalid update plan.'
    }
    $Target = Join-Path $Root $plan.target
    $Source = Join-Path $Stage 'ladder.exe'
    foreach ($path in @($Source, $Target)) {
        if ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Executable cannot be a link.'
        }
    }
    [IO.File]::WriteAllText((Join-Path $Stage 'ready'), 'ready')
    try { $parent = [Diagnostics.Process]::GetProcessById($plan.parent) } catch [ArgumentException] { $parent = $null }
    if ($null -ne $parent) {
        try {
            if (-not $parent.WaitForExit(60000)) { throw 'Ladder did not exit; update was cancelled.' }
        } finally { $parent.Dispose() }
    }
    $commit = Join-Path $Stage 'commit'
    if (-not (Test-Path -LiteralPath $commit -PathType Leaf) -or
        [IO.File]::ReadAllText($commit) -cne 'commit') {
        throw 'Ladder did not confirm the update.'
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    $inputFile = [IO.File]::OpenRead($Source)
    try { $hash = [BitConverter]::ToString($hasher.ComputeHash($inputFile)).Replace('-', '').ToLowerInvariant() }
    finally { $inputFile.Dispose(); $hasher.Dispose() }
    if ($hash -cne $plan.sha256) {
        throw 'Update checksum does not match.'
    }
    [IO.File]::Replace($Source, $Target, $Backup, $true)
    $Replaced = $true
    $restarted = Start-Process -FilePath $Target -WorkingDirectory $Root -WindowStyle Hidden -PassThru
    try {
        if ($restarted.WaitForExit(3000) -and $restarted.ExitCode -ne 0) { throw 'Updated Ladder could not start.' }
    } finally { $restarted.Dispose() }
} catch {
    if ($Replaced -and (Test-Path -LiteralPath $Backup -PathType Leaf)) {
        try {
            [IO.File]::Replace($Backup, $Target, (Join-Path $Stage 'failed.exe'), $true)
            Start-Process -FilePath $Target -WorkingDirectory $Root -WindowStyle Hidden | Out-Null
        } catch { }
    }
    # Do not write the exception: it may contain local paths or network information.
    [IO.File]::WriteAllText((Join-Path $Root 'update-error.log'), 'Ladder update failed. Your configuration was preserved. Retry the update or replace ladder.exe manually.')
    exit 1
}
# Stage was validated as a direct child with the reserved update prefix.
try {
    Remove-Item -LiteralPath $Stage -Recurse -Force
    $failureLog = Join-Path $Root 'update-error.log'
    if (Test-Path -LiteralPath $failureLog) { Remove-Item -LiteralPath $failureLog -Force }
} catch { }
