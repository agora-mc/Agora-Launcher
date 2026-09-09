param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$file = Get-Item -LiteralPath $FilePath
if ($file.PSIsContainer -or $file.Extension -notin @('.exe', '.msi', '.dll')) {
    throw "Unsupported signing target: $FilePath"
}
foreach ($variable in @('AGORA_SIGNTOOL_PATH', 'AGORA_SIGNING_DLIB', 'AGORA_SIGNING_METADATA')) {
    $value = [Environment]::GetEnvironmentVariable($variable)
    if (-not $value -or -not (Test-Path -LiteralPath $value -PathType Leaf)) {
        throw "Missing signing setup: $variable"
    }
}

# Array arguments preserve paths with spaces and shell metacharacters. Tauri calls
# this hook for the application and installers before producing updater signatures.
& $env:AGORA_SIGNTOOL_PATH sign /v /fd SHA256 /td SHA256 `
    /tr 'http://timestamp.acs.microsoft.com' /d 'Agora Launcher' `
    /dlib $env:AGORA_SIGNING_DLIB /dmdf $env:AGORA_SIGNING_METADATA $file.FullName
if ($LASTEXITCODE -ne 0) { throw "Authenticode signing failed for $($file.Name) (exit $LASTEXITCODE)." }

& $env:AGORA_SIGNTOOL_PATH verify /pa /all /v $file.FullName
if ($LASTEXITCODE -ne 0) { throw "Authenticode verification failed for $($file.Name) (exit $LASTEXITCODE)." }

$signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
    throw "A valid, timestamped Authenticode signature is required for $($file.Name)."
}
Write-Host "Verified timestamped Windows signature: $($file.Name)"
