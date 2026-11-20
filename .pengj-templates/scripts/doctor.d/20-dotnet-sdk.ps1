# WinUI 3 参考截图宿主（tools/winui-reference）以 net8.0-windows 为目标框架构建，
# 机器上只有运行时不够，必须能找到 .NET 8+ SDK，`just parity-capture` 才跑得起来。
#
# 这里只做检测：把带 SDK 的 dotnet 排到 PATH 更前面需要管理员权限，doctor 不替用户
# 改环境变量，只把「首个 dotnet 没 SDK、后面某个有」这种最常见的坑指出来。
#
# 候选目录同时取自当前进程 PATH 与注册表里的持久化 PATH：刚装完 SDK 的终端还没刷新，
# 只看进程环境会漏掉新装的目录。

$minMajor = 8

# 返回该 dotnet 可用的、主版本号 >= $minMajor 的 SDK 版本；没有就返回 $null
function Get-DotnetSdkVersion($exe) {
    foreach ($line in @(& $exe --list-sdks 2>$null)) {
        $version = ($line -split '\s+')[0]
        if ($version -match '^\d+\.\d+\.\d+$' -and [int]($version.Split('.')[0]) -ge $minMajor) {
            return $version
        }
    }
    return $null
}

# 实际会被命中的 dotnet 排在最前，后面按目录顺序补齐其余候选
$ordered = @()
$resolved = Get-Command dotnet -ErrorAction SilentlyContinue
if ($resolved) {
    $ordered += $resolved.Source
}
$dirs = @(
    ($env:PATH -split ';')
    ([Environment]::GetEnvironmentVariable('Path', 'User') -split ';')
    ([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';')
) | Where-Object { $_ } | Select-Object -Unique
foreach ($dir in $dirs) {
    $exe = Join-Path $dir 'dotnet.exe'
    if (Test-Path $exe) {
        $ordered += (Resolve-Path $exe).Path
    }
}
$candidates = @($ordered | Select-Object -Unique)

if ($candidates.Count -eq 0) {
    Write-Host "dotnet not found: tools/winui-reference needs the .NET $minMajor+ SDK (runtime alone is not enough)"
    exit 1
}

$first = $candidates[0]
$firstSdk = Get-DotnetSdkVersion $first
if ($firstSdk) {
    Write-Host "dotnet SDK $firstSdk ($first)"
    exit 0
}

# PATH 首个 dotnet 不合格：继续往后找，命中时给出的提示才是可直接照做的
$usable = $null
foreach ($exe in ($candidates | Select-Object -Skip 1)) {
    $version = Get-DotnetSdkVersion $exe
    if ($version) {
        $usable = [pscustomobject]@{ Path = $exe; Version = $version }
        break
    }
}

if ($usable) {
    Write-Host "first dotnet ($first) has no .NET $minMajor+ SDK, but $($usable.Path) has $($usable.Version); put that directory ahead of it on PATH"
} else {
    Write-Host "no .NET $minMajor+ SDK found (first dotnet: $first); install the SDK to build tools/winui-reference"
}
exit 1
