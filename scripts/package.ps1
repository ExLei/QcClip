#Requires -Version 7.0

[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string[]]$Arch,

    [ValidateSet('nsis', 'app', 'dmg', 'deb', 'appimage')]
    [string[]]$Formats,

    [ValidateSet('installer', 'portable', 'both')]
    [string]$PackageType,

    [string]$RuntimeDirectory,

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot

function Invoke-Cargo {
    param([string[]]$Arguments)

    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo 命令执行失败，退出码：$LASTEXITCODE"
    }
}

function Assert-WindowsArchitecture {
    param([string]$Path, [string]$Architecture)

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        $signature = $reader.ReadUInt32()
        $machine = $reader.ReadUInt16()
        $expectedMachine = if ($Architecture -eq 'x64') { 0x8664 } else { 0xaa64 }
        if ($signature -ne 0x4550 -or $machine -ne $expectedMachine) {
            throw "程序或运行库不是目标架构 $Architecture 的 PE 文件：$Path"
        }
    } finally {
        $stream.Dispose()
    }
}

function Find-WindowsRuntime {
    param([string]$Architecture)

    $directory = $RuntimeDirectory
    if (!$directory) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        if (!(Test-Path -LiteralPath $vswhere -PathType Leaf)) {
            throw '找不到 Visual Studio Installer，请使用 -RuntimeDirectory 指定对应架构的 MSVC CRT 可再分发目录'
        }
        $installations = & $vswhere -products '*' -property installationPath
        if ($LASTEXITCODE -ne 0) {
            throw '无法查询 Visual Studio 安装目录'
        }
        $candidates = foreach ($installation in $installations) {
            $redistRoot = Join-Path $installation 'VC/Redist/MSVC'
            if (Test-Path -LiteralPath $redistRoot -PathType Container) {
                Get-ChildItem -LiteralPath $redistRoot -Directory |
                    Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
                    ForEach-Object {
                        $crtDirectory = Join-Path $_.FullName "$Architecture/Microsoft.VC143.CRT"
                        if (Test-Path -LiteralPath $crtDirectory -PathType Container) {
                            [pscustomobject]@{ Version = [version]$_.Name; Directory = $crtDirectory }
                        }
                    }
            }
        }
        $directory = ($candidates | Sort-Object Version -Descending | Select-Object -First 1).Directory
    }
    if (!$directory -or !(Test-Path -LiteralPath (Join-Path $directory 'vcruntime140.dll') -PathType Leaf)) {
        throw "找不到 $Architecture 的 MSVC 可再分发运行库，请安装对应 Visual Studio C++ 组件，或使用 -RuntimeDirectory 指定 Microsoft.VC143.CRT 目录"
    }
    $runtimeFiles = @(Get-Item -LiteralPath (Join-Path $directory 'vcruntime140.dll'))
    foreach ($runtimeFile in $runtimeFiles) {
        Assert-WindowsArchitecture -Path $runtimeFile.FullName -Architecture $Architecture
    }
    return $runtimeFiles
}

Push-Location -LiteralPath $projectRoot
try {
    $rustInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0) {
        throw '无法读取 Rust 工具链信息'
    }
    $hostLine = $rustInfo | Select-String -Pattern '^host: (.+)$'
    if (!$hostLine) {
        throw '无法确定 Rust 工具链的主机目标'
    }
    $hostTarget = $hostLine.Matches[0].Groups[1].Value

    if ($hostTarget -match '-pc-windows-msvc$') {
        $platform = 'windows'
        $targetSuffix = 'pc-windows-msvc'
        $allowedFormats = @('nsis')
        $defaultFormats = @('nsis')
        $binaryName = 'QcClip.exe'
    } elseif ($hostTarget -match '-apple-darwin$') {
        $platform = 'macos'
        $targetSuffix = 'apple-darwin'
        $allowedFormats = @('app', 'dmg')
        $defaultFormats = @('dmg')
        $binaryName = 'QcClip'
    } elseif ($hostTarget -match '-unknown-linux-gnu$') {
        $platform = 'linux'
        $targetSuffix = 'unknown-linux-gnu'
        $allowedFormats = @('deb', 'appimage')
        $defaultFormats = @('deb', 'appimage')
        $binaryName = 'QcClip'
    } else {
        throw "不支持当前主机目标：$hostTarget，请使用 Windows MSVC、macOS 或 Linux GNU 工具链"
    }

    if (!$PackageType) {
        $PackageType = if ($platform -eq 'windows') { 'both' } else { 'installer' }
    }
    if ($platform -ne 'windows' -and $PackageType -ne 'installer') {
        throw '便携 ZIP 当前仅支持 Windows；macOS 和 Linux 的便携发布需完成平台适配后再接入'
    }
    $createInstaller = $PackageType -ne 'portable'
    $createPortable = $PackageType -ne 'installer'
    if ($createInstaller) {
        & cargo packager --version
        if ($LASTEXITCODE -ne 0) {
            throw '请先安装打包工具：cargo install cargo-packager --version 0.11.8 --locked'
        }
    }

    if (!$Arch) {
        if ($hostTarget.StartsWith('x86_64-')) {
            $Arch = @('x64')
        } elseif ($hostTarget.StartsWith('aarch64-')) {
            $Arch = @('arm64')
        } else {
            throw "不支持当前主机架构：$hostTarget"
        }
    }
    if (!$Formats) {
        $Formats = $defaultFormats
    }
    foreach ($format in $Formats) {
        if ($format -notin $allowedFormats) {
            throw "$platform 不支持 $format，允许的格式：$($allowedFormats -join ', ')"
        }
    }

    $metadataJson = & cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) {
        throw '无法读取 Cargo 项目元数据'
    }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'QcClip' }
    if (!$package) {
        throw 'Cargo 元数据中找不到 QcClip 项目'
    }

    foreach ($architecture in ($Arch | Select-Object -Unique)) {
        $rustArch = if ($architecture -eq 'x64') { 'x86_64' } else { 'aarch64' }
        $target = "$rustArch-$targetSuffix"
        $binaryDirectory = Join-Path $metadata.target_directory 'release'
        $buildArguments = @('build', '--release', '--locked', '--bin', 'QcClip')
        if ($target -ne $hostTarget) {
            $binaryDirectory = Join-Path $metadata.target_directory "$target/release"
            $buildArguments += @('--target', $target)
        }
        $outputDirectory = Join-Path $metadata.target_directory "packages/$target"

        Write-Host "正在打包 $platform / $architecture，Rust 目标：$target"
        if (!$SkipBuild) {
            Invoke-Cargo -Arguments $buildArguments
        }
        $binaryPath = Join-Path $binaryDirectory $binaryName
        if (!(Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
            throw "找不到待打包程序：$binaryPath，请先构建对应目标或移除 -SkipBuild"
        }

        $runtimeFiles = @()
        if ($platform -eq 'windows') {
            Assert-WindowsArchitecture -Path $binaryPath -Architecture $architecture
            $runtimeFiles = @(Find-WindowsRuntime -Architecture $architecture)
        }

        # 直接传入配置，避免打包器忽略 --binaries-dir 后选错架构目录。
        $config = $package.metadata.packager | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
        $config['name'] = $package.name
        $config['version'] = $package.version
        $config['binaries'] = @(@{ path = 'QcClip'; main = $true })
        $config['binariesDir'] = $binaryDirectory
        $config['outDir'] = $outputDirectory
        $config['targetTriple'] = $target
        $config['formats'] = @($Formats)
        if ($runtimeFiles.Count -gt 0) {
            $config['resources'] = @($config['resources'] | Where-Object { $null -ne $_ }) + @($runtimeFiles.FullName)
        }
        if ($createInstaller) {
            $configJson = ConvertTo-Json -InputObject $config -Depth 20 -Compress
            Invoke-Cargo -Arguments @('packager', '--config', $configJson)
        }
        if ($createPortable) {
            New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
            $stageDirectory = Join-Path $outputDirectory ".portable-$([guid]::NewGuid().ToString('N'))"
            $portableRoot = Join-Path $stageDirectory 'QcClip'
            $archivePath = Join-Path $outputDirectory "QcClip_$($package.version)_${architecture}-portable.zip"
            try {
                New-Item -ItemType Directory -Path $portableRoot | Out-Null
                Copy-Item -LiteralPath $binaryPath -Destination $portableRoot
                foreach ($runtimeFile in $runtimeFiles) {
                    Copy-Item -LiteralPath $runtimeFile.FullName -Destination $portableRoot
                }
                New-Item -ItemType File -Path (Join-Path $portableRoot 'QcClip.portable') | Out-Null
                Compress-Archive -LiteralPath $portableRoot -DestinationPath $archivePath -CompressionLevel Optimal -Force
                Write-Host "便携版已生成：$archivePath"
            } finally {
                # 只清理本次打包创建、且位于产物目录内的暂存目录。
                if (Test-Path -LiteralPath $stageDirectory -PathType Container) {
                    $resolvedOutput = (Resolve-Path -LiteralPath $outputDirectory).Path
                    $resolvedStage = (Resolve-Path -LiteralPath $stageDirectory).Path
                    if ((Split-Path -Parent $resolvedStage) -ne $resolvedOutput) {
                        throw "暂存目录不在产物目录内，拒绝清理：$resolvedStage"
                    }
                    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
                }
            }
        }
        Write-Host "打包完成：$outputDirectory"
    }
} finally {
    Pop-Location
}
