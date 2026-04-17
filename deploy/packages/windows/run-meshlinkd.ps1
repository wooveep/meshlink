param(
    [string]$ConfigPath = (Join-Path $PSScriptRoot "client.toml")
)

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this script from an elevated PowerShell session."
}

$binaryPath = Join-Path $PSScriptRoot "meshlinkd.exe"
if (-not (Test-Path -Path $binaryPath)) {
    throw "meshlinkd.exe was not found next to this script."
}

foreach ($runtimeAsset in @("tunnel.dll", "wireguard.dll", "wintun.dll")) {
    $runtimePath = Join-Path $PSScriptRoot $runtimeAsset
    if (-not (Test-Path -Path $runtimePath)) {
        throw "$runtimeAsset was not found next to this script."
    }
}

if (-not (Test-Path -Path $ConfigPath)) {
    $examplePath = Join-Path $PSScriptRoot "client.example.toml"
    if (Test-Path -Path $examplePath) {
        throw "Config file not found: $ConfigPath. Copy client.example.toml to client.toml and update it before starting MeshLink."
    }
    throw "Config file not found: $ConfigPath"
}

& $binaryPath --config $ConfigPath
exit $LASTEXITCODE
