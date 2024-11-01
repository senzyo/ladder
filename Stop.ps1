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