[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release"
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$stageRoot = Join-Path $PSScriptRoot "stage"
$serviceOutput = Join-Path $stageRoot "sensor-service"
$desktopExe = Join-Path $projectRoot "target\release\desktop-assistant.exe"
$serviceExe = Join-Path $serviceOutput "ZZHSensorService.exe"
$notices = Join-Path $PSScriptRoot "ThirdPartyNotices.txt"
$appIcon = Join-Path $projectRoot "apps\desktop\assets\windows\zzh-assistant.ico"
$installerProject = Join-Path $PSScriptRoot "ZZHDesktopAssistant.Installer.wixproj"
$bundleProject = Join-Path $PSScriptRoot "ZZHDesktopAssistant.Bundle.wixproj"
$pawnIoSetup = Join-Path $PSScriptRoot "PawnIO_setup_2.2.0.exe"
$expectedPawnIoHash = "1F519A22E47187F70A1379A48CA604981C4FCF694F4E65B734AAA74A9FBA3032"
$installerOutput = Join-Path $PSScriptRoot "bin\x64\$Configuration\ZZHDesktopAssistant-Setup.msi"
$bundleOutput = Join-Path $PSScriptRoot "bin\x64\$Configuration\ZZHDesktopAssistant-Setup.exe"
$rootBundle = Join-Path $projectRoot "ZZHDesktopAssistant-Setup.exe"

if (-not (Test-Path -LiteralPath $pawnIoSetup)) {
    throw "Pinned PawnIO setup is missing: $pawnIoSetup"
}
$pawnIoHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pawnIoSetup).Hash
if ($pawnIoHash -ne $expectedPawnIoHash) {
    throw "PawnIO setup hash mismatch. Expected $expectedPawnIoHash, got $pawnIoHash."
}
$pawnIoSignature = Get-AuthenticodeSignature -LiteralPath $pawnIoSetup
if ($pawnIoSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "PawnIO setup signature is not valid: $($pawnIoSignature.StatusMessage)"
}

New-Item -ItemType Directory -Path $serviceOutput -Force | Out-Null

dotnet publish (Join-Path $projectRoot "services\sensor-service\ZZHSensorService.csproj") `
    --configuration $Configuration `
    --runtime win-x64 `
    --self-contained true `
    --output $serviceOutput `
    -p:PublishSingleFile=true `
    -p:PublishTrimmed=false `
    -p:NuGetAudit=false
if ($LASTEXITCODE -ne 0) { throw "Sensor service publish failed." }

cargo build --manifest-path (Join-Path $projectRoot "Cargo.toml") `
    --package desktop-assistant `
    --release `
    --offline
if ($LASTEXITCODE -ne 0) { throw "Desktop assistant build failed." }

dotnet build $installerProject `
    --configuration $Configuration `
    -p:DesktopExe=$desktopExe `
    -p:SensorServiceExe=$serviceExe `
    -p:ThirdPartyNotices=$notices `
    -p:AppIcon=$appIcon `
    -p:NuGetAudit=false
if ($LASTEXITCODE -ne 0) { throw "MSI build failed." }

if (-not (Test-Path -LiteralPath $installerOutput)) {
    throw "Expected MSI was not created: $installerOutput"
}

dotnet build $bundleProject `
    --configuration $Configuration `
    -p:MainMsi=$installerOutput `
    -p:PawnIoSetup=$pawnIoSetup `
    -p:AppIcon=$appIcon `
    -p:NuGetAudit=false
if ($LASTEXITCODE -ne 0) { throw "Installer bundle build failed." }

if (-not (Test-Path -LiteralPath $bundleOutput)) {
    throw "Expected installer bundle was not created: $bundleOutput"
}

Copy-Item -LiteralPath $desktopExe -Destination (Join-Path $projectRoot "desktop-assistant.exe") -Force
Copy-Item -LiteralPath $bundleOutput -Destination $rootBundle -Force

Get-FileHash -Algorithm SHA256 $desktopExe, $serviceExe, $rootBundle |
    Select-Object Path, Hash
