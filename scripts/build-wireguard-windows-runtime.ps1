param(
    [string]$RuntimeVersion = "v0.3.17",
    [string]$WireGuardWindowsRef = "v0.3.17",
    [string]$WireGuardWindowsRepo = "https://git.zx2c4.com/wireguard-windows",
    [Parameter(Mandatory = $true)]
    [string]$WireGuardDllPath,
    [string]$TargetArch = "amd64",
    [string]$WorkRoot = "$PSScriptRoot\..\dist\windows-runtime-src"
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$stageDir = Join-Path $repoRoot "deploy\packages\windows\runtime\$RuntimeVersion\$TargetArch"
$sourceDir = Join-Path $WorkRoot "wireguard-windows-$WireGuardWindowsRef"
$buildDir = Join-Path $sourceDir "embeddable-dll-service"
$tunnelDll = Join-Path $buildDir "$TargetArch\tunnel.dll"

New-Item -ItemType Directory -Force -Path $WorkRoot | Out-Null
New-Item -ItemType Directory -Force -Path $stageDir | Out-Null

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git is required to fetch wireguard-windows sources."
}

if (-not (Test-Path $sourceDir)) {
    git clone --branch $WireGuardWindowsRef --depth 1 $WireGuardWindowsRepo $sourceDir
}

Push-Location $buildDir
try {
    cmd.exe /c build.bat
} finally {
    Pop-Location
}

if (-not (Test-Path $tunnelDll)) {
    throw "Expected tunnel.dll was not produced at: $tunnelDll"
}

if (-not (Test-Path $WireGuardDllPath)) {
    throw "wireguard.dll was not found at: $WireGuardDllPath"
}

Copy-Item -Force $tunnelDll (Join-Path $stageDir "tunnel.dll")
Copy-Item -Force $WireGuardDllPath (Join-Path $stageDir "wireguard.dll")

Write-Host "Staged Windows runtime assets into $stageDir"
