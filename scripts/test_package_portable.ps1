# Offline packaging checks; signatures are mocked, no real signed release is made.
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('agora-portable-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testRoot | Out-Null
$exe = Join-Path $testRoot 'test desktop.exe'
Set-Content -LiteralPath $exe -Value 'fixture executable' -Encoding ascii
$cli = Join-Path $testRoot 'agora.exe'
Set-Content -LiteralPath $cli -Value 'fixture cli' -Encoding ascii
function Get-AuthenticodeSignature {
    param([string]$LiteralPath)
    [pscustomobject]@{ Status = 'Valid'; TimeStamperCertificate = 'fixture' }
}
$output = Join-Path $testRoot 'out'
& "$PSScriptRoot/package_portable.ps1" -Executable $exe -Cli $cli -Version v1.2.3 -OutputDirectory $output
$zip = Join-Path $output 'agora-desktop-v1.2.3-windows-x64-portable.zip'
$unpacked = Join-Path $testRoot 'unpacked'
Expand-Archive -LiteralPath $zip -DestinationPath $unpacked
foreach ($file in @('Agora Launcher.exe', 'agora.exe', 'portable.txt', 'README.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $unpacked $file))) { throw "Missing $file" }
}
$marker = [IO.File]::ReadAllBytes((Join-Path $unpacked 'portable.txt'))
if ($marker[0] -ne 100 -or (Get-Content -LiteralPath (Join-Path $unpacked 'portable.txt')).Trim() -ne 'data') {
    throw 'Marker must name data without a BOM'
}
if ((Get-FileHash -LiteralPath $exe).Hash -ne (Get-FileHash -LiteralPath (Join-Path $unpacked 'Agora Launcher.exe')).Hash) {
    throw 'Packaged executable changed'
}
if ((Get-FileHash -LiteralPath $cli).Hash -ne (Get-FileHash -LiteralPath (Join-Path $unpacked 'agora.exe')).Hash) {
    throw 'Packaged CLI changed'
}
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not (Get-Content -LiteralPath "$zip.sha256").StartsWith("$hash  ")) { throw 'Checksum mismatch' }
function Get-AuthenticodeSignature {
    param([string]$LiteralPath)
    [pscustomobject]@{ Status = 'NotSigned'; TimeStamperCertificate = $null }
}
$rejected = $false
try {
    & "$PSScriptRoot/package_portable.ps1" -Executable $exe -Cli $cli -Version v1.2.4 -OutputDirectory $output
} catch { $rejected = $_.Exception.Message -like '*valid timestamped Authenticode*' }
if (-not $rejected) { throw 'Unsigned executable was not rejected' }
# A signed app with an unsigned CLI must be rejected too.
function Get-AuthenticodeSignature {
    param([string]$LiteralPath)
    if ($LiteralPath -like '*agora.exe') { return [pscustomobject]@{ Status = 'NotSigned'; TimeStamperCertificate = $null } }
    [pscustomobject]@{ Status = 'Valid'; TimeStamperCertificate = 'fixture' }
}
$rejected = $false
try {
    & "$PSScriptRoot/package_portable.ps1" -Executable $exe -Cli $cli -Version v1.2.5 -OutputDirectory $output
} catch { $rejected = $_.Exception.Message -like '*agora.exe*' }
if (-not $rejected) { throw 'Unsigned CLI was not rejected' }
Write-Output 'Portable packaging checks passed (mock signature).'
