$ESC = [char]27
$Black = "$ESC[90m"
$Red = "$ESC[91m"       # [错误]
$Green = "$ESC[92m"     # [成功]
$Yellow = "$ESC[93m"    # [警告]
$Blue = "$ESC[94m"
$Magenta = "$ESC[95m"
$Cyan = "$ESC[96m"      # [提示]
$White = "$ESC[97m"
$NC = "$ESC[0m"         # 无颜色

# 检测是否以管理员身份运行此脚本
$currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
$isAdmin = $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "${Red}[错误]${NC} 请以管理员身份运行此脚本"
    exit 1
}

# 停止运行 sing-box 和 xray
$Process = @("sing-box", "xray")
foreach ($P in $Process) {
    if (Get-Process $P -ErrorAction SilentlyContinue) {
        Stop-Process -Name $P -Force
        Write-Host "${Green}[成功]${NC} $P 已停止"
    } else {
        Write-Host "${Yellow}[警告]${NC} $P 未在运行"
    }
}
Clear-DnsClientCache
Start-Sleep -Seconds 1

# ========== 公共函数 [开始] ==========
# 校验 Hash
function VerifyHash {
    $Digest = $Response.assets | Where-Object { $_.name -eq "$FileName" } | Select-Object -ExpandProperty digest
    $RemoteHash = $Digest.Split(':')[-1]
    $LocalHash = (Get-FileHash $FilePath -Algorithm SHA256).Hash.ToLower()
    if ($RemoteHash -eq $LocalHash) {
        Write-Host "${Green}[成功]${NC} 文件完整性检查通过"
        return $true
    } else {
        Write-Host "${Red}[错误]${NC} 文件已损坏"
        return $false
    }
}

# 升级
function Upgrade {
    $Url = $Response.assets | Where-Object { $_.name -eq "$FileName" } | Select-Object -ExpandProperty browser_download_url
    do {
        if (Test-Path -Path $FilePath) {
            Remove-Item -Force $FilePath
        }
        Write-Host "${Cyan}[提示]${NC} 正在下载"
        Invoke-WebRequest -OutFile $FilePath -Uri "https://v6.gh-proxy.org/$Url"
        $Correct = VerifyHash
        if ($Correct) {
            $script:Cover = $true
        } else {
            Write-Host "${Cyan}[提示]${NC} 正在重新下载"
            Start-Sleep -Seconds 1
        }
    } until ($Correct)
}

# 检查更新
function CheckUpdate ($ExeName, $VersionArg) {
    $LocalVersionStr = "0.0.0"
    if (Test-Path -Path $ExePath) {
        $VersionOutput = (& $ExePath $VersionArg) 2>&1
        $VersionText = $VersionOutput -join " "
        if ($VersionText -match "([\d.]+)") {
            $LocalVersionStr = $Matches[1]
        }
    }
    $LocalVersionObj = [System.Version]$LocalVersionStr
    $RemoteVersionObj = [System.Version]$RemoteVersionStr
    if ($RemoteVersionObj -gt $LocalVersionObj) {
        Write-Host "${Yellow}[警告]${NC} 有新版本: ${Yellow}$LocalVersionStr${NC} -> ${Green}$RemoteVersionStr${NC}"
        Upgrade
    }
    else {
        Write-Host "${Green}[成功]${NC} 已是最新: $ExeName ${Green}$LocalVersionStr${NC}"
    }
}
# ========== 公共函数 [结束] ==========

$WorkDir = "$env:USERPROFILE\Apps\sing-box_with_xray"

# ========== 更新 sing-box [开始] ==========
Write-Host "${Cyan}[提示]${NC} 正在检查更新 sing-box"
$ExePath = "$WorkDir\sing-box.exe"
$Response = Invoke-RestMethod -Uri "https://api.github.com/repos/SagerNet/sing-box/releases/latest" -Method Get
$TagName = $Response.tag_name
$RemoteVersionStr = $TagName.TrimStart('v')
$FileName = "sing-box-$RemoteVersionStr-windows-amd64.zip"
$FilePath = "$WorkDir\$FileName"
$Folder = $FileName -replace '\.zip$', ''

$script:Cover = $false
CheckUpdate 'sing-box' 'version'

# 解压缩并覆盖 sing-box
if ($script:Cover) {
    Expand-Archive -Path $FilePath -DestinationPath $WorkDir -Force
    Move-Item -Path "$WorkDir\$Folder\sing-box.exe" -Destination "$ExePath" -Force
    Remove-Item -Force -Recurse "$WorkDir\$Folder","$FilePath"
}
# ========== 更新 sing-box [结束] ==========

# ========== 更新 xray [开始] ==========
Write-Host "${Cyan}[提示]${NC} 正在检查更新 xray"
$ExePath = "$WorkDir\xray.exe"
$Response = Invoke-RestMethod -Uri "https://api.github.com/repos/XTLS/Xray-core/releases/latest" -Method Get
$TagName = $Response.tag_name
$RemoteVersionStr = $TagName.TrimStart('v')
$FileName = "Xray-windows-64.zip"
$FilePath = "$WorkDir\$FileName"
$Folder = $FileName -replace '\.zip$', ''

$script:Cover = $false
CheckUpdate 'xray' 'version'

# 解压缩并覆盖 xray
if ($script:Cover) {
    Expand-Archive -Path $FilePath -DestinationPath "$WorkDir\$Folder" -Force
    Move-Item -Path "$WorkDir\$Folder\xray.exe" -Destination "$ExePath" -Force
    Remove-Item -Force -Recurse "$WorkDir\$Folder","$FilePath"
}
# ========== 更新 xray [结束] ==========

# ========== 更新 jq [开始] ==========
Write-Host "${Cyan}[提示]${NC} 正在检查更新 jq"
$ExePath = "$WorkDir\jq.exe"
$Response = Invoke-RestMethod -Uri "https://api.github.com/repos/jqlang/jq/releases/latest" -Method Get
$TagName = $Response.tag_name
$RemoteVersionStr = $TagName.TrimStart('jq-')
$FileName = "jq-windows-amd64.exe"
$FilePath = "$WorkDir\$FileName"

$script:Cover = $false
CheckUpdate 'jq' '--version'

# 覆盖 jq
if ($script:Cover) {
    Move-Item -Path "$FilePath" -Destination "$ExePath" -Force
}
# ========== 更新 jq [结束] ==========

pause