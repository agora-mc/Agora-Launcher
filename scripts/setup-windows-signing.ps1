# Used only on the ephemeral Windows release runner after azure/login.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows -or -not $env:RUNNER_TEMP -or -not $env:GITHUB_ENV) {
    throw 'Windows signing setup must run on a Windows GitHub Actions runner.'
}

$signingDir = Join-Path $env:RUNNER_TEMP 'agora-windows-signing'
New-Item -ItemType Directory -Force -Path $signingDir | Out-Null

# Pin both version and content; verify the NuGet package signature before extraction.
$clientVersion = '1.0.128'
$clientSha256 = '74bd7d27e6ce1051409c38d9b46bc8df0400ecd643d51ffbf2ac00869061e40b'
$package = Join-Path $signingDir 'client.nupkg'
Invoke-WebRequest -Uri "https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/$clientVersion/microsoft.artifactsigning.client.$clientVersion.nupkg" -OutFile $package
if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ne $clientSha256) {
    throw 'Microsoft Artifact Signing client SHA-256 mismatch.'
}
& dotnet nuget verify $package --all
if ($LASTEXITCODE -ne 0) { throw 'Microsoft Artifact Signing client package signature verification failed.' }

$clientDir = Join-Path $signingDir 'client'
[IO.Compression.ZipFile]::ExtractToDirectory($package, $clientDir, $true)
$dlib = Join-Path $clientDir 'bin/x64/Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path -LiteralPath $dlib -PathType Leaf)) {
    throw 'The signing client package does not contain the expected x64 signing library.'
}

# Use the signed Windows SDK tool supplied by the GitHub-hosted runner.
$sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$signtool = Get-ChildItem -LiteralPath $sdkBin -Directory |
    Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64/signtool.exe' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (-not $signtool) { throw 'No x64 Windows SDK signtool.exe found.' }
$toolSignature = Get-AuthenticodeSignature -LiteralPath $signtool
if ($toolSignature.Status -ne 'Valid' -or $toolSignature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
    throw 'The Windows SDK signing tool does not have a valid Microsoft signature.'
}

$metadataPath = Join-Path $signingDir 'metadata.json'
@{
    Endpoint = 'https://wus2.codesigning.azure.net'
    CodeSigningAccountName = 'Agora-MC-Launcher'
    CertificateProfileName = 'Agora-MC-Launcher'
    # Authenticate only through the short-lived azure/login CLI session.
    ExcludeCredentials = @(
        'EnvironmentCredential', 'WorkloadIdentityCredential', 'ManagedIdentityCredential',
        'SharedTokenCacheCredential', 'VisualStudioCredential', 'VisualStudioCodeCredential',
        'AzurePowerShellCredential', 'AzureDeveloperCliCredential', 'InteractiveBrowserCredential'
    )
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $metadataPath -Encoding utf8NoBOM

$configPath = Join-Path $signingDir 'tauri-signing.json'
$signScript = Join-Path $PSScriptRoot 'sign-windows.ps1'
@{
    bundle = @{
        windows = @{
            signCommand = @{
                cmd = 'pwsh'
                args = @('-NoProfile', '-NonInteractive', '-File', $signScript, '-FilePath', '%1')
            }
        }
    }
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configPath -Encoding utf8NoBOM

@(
    "AGORA_SIGNTOOL_PATH=$signtool"
    "AGORA_SIGNING_DLIB=$dlib"
    "AGORA_SIGNING_METADATA=$metadataPath"
    "AGORA_WINDOWS_SIGNING_CONFIG=$configPath"
) | Out-File -LiteralPath $env:GITHUB_ENV -Append -Encoding utf8
