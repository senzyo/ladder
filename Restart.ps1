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

# ========== 检测并卸载残留的 TUN 网卡 [开始] ==========
# 枚举所有网络适配器类型的 PnP 设备
try {
    $allNetDevices = Get-PnpDevice -Class Net -ErrorAction Stop
} catch {
    Write-Host "${Red}[错误]${NC} 获取 PnP 设备失败"
    exit
}

# 过滤并识别残留的 Wintun 设备
$orphanedWintunDevices = $allNetDevices | Where-Object {
    # 识别是否为 Wintun 相关设备
    (($_.Service -eq "wintun") -or
     ($_.InstanceId -like "*WINTUN*") -or
     ($_.FriendlyName -like "*Wintun*")) -and
    # 识别是否为“孤立/离线/异常”状态
    (($_.Present -eq $false) -or
     ($_.Status -eq "Unknown") -or
     ($_.Status -eq "Error"))
}

# 判断、卸载、展示
if ($orphanedWintunDevices) {
    # 强制卸载孤立设备
    $successCount = 0
    $failCount = 0
    foreach ($dev in $orphanedWintunDevices) {
        $pnpOutput = pnputil /remove-device $dev.InstanceId 2>&1
        if ($LASTEXITCODE -eq 0) {
            $successCount++
        } else {
            $failCount++
        }
    }

    # 重新扫描硬件以刷新设备管理器
    pnputil /scan-devices | Out-Null

    # 输出统计报告
    Write-Host "${Green}[成功]${NC} 清理残留 TUN 网卡 $successCount 个"
    if ($failCount -gt 0) {
        Write-Host "${Yellow}[警告]${NC} 清理残留 TUN 网卡失败 $failCount 个"
    }
}
# ========== 检测并卸载残留的 TUN 网卡 [结束] ==========

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

$WorkDir = "$env:USERPROFILE\Apps\sing-box_with_xray"
$ConfigPath = "$WorkDir\sing-box.json"
$TempPath = "$WorkDir\sing-box.json.temp"
$RandomHex = -join (1..3 | ForEach-Object { "{0:x2}" -f (Get-Random -Min 0 -Max 256) })
if (Test-Path $ConfigPath) {
    $JsonResult = & $WorkDir\jq.exe --arg new_name "$RandomHex" '(.inbounds[] | select(.type == \"tun\") | .interface_name) = $new_name' $ConfigPath 2>$null
    if ($LASTEXITCODE -eq 0 -and $JsonResult) {
        [System.IO.File]::WriteAllLines($TempPath, $JsonResult)
        Move-Item -Path $TempPath -Destination $ConfigPath -Force
        Write-Host "${Cyan}[提示]${NC} 正在启动 sing-box"
        Start-Process -FilePath "$WorkDir\sing-box.exe" -ArgumentList "run -D $WorkDir -c $ConfigPath" -WindowStyle Hidden
    } else {
        Write-Host "${Red}[错误]${NC} 随机化 TUN 网卡名称失败"
        pause
        exit
    }
} else {
    Write-Host "${Red}[错误]${NC} 文件不存在: $ConfigPath"
    pause
    exit
}

Write-Host "${Cyan}[提示]${NC} 正在启动 xray"
$ConfigPath = "$WorkDir\xray.json"
if (Test-Path $ConfigPath) {
    Start-Process -FilePath "$WorkDir\xray.exe" -ArgumentList "run -c $ConfigPath" -WindowStyle Hidden
} else {
    Write-Host "${Red}[错误]${NC} 文件不存在: $ConfigPath"
    pause
    exit
}

Start-Sleep -Seconds 2
foreach ($P in $Process) {
    if (Get-Process $P -ErrorAction SilentlyContinue) {
        Write-Host "${Green}[成功]${NC} $P 正在运行"
    } else {
        Write-Host "${Red}[错误]${NC} $P 未在运行"
    }
}
Start-Sleep -Seconds 1