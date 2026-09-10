param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^v?\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$') {
    throw "Invalid release version: $Version"
}
$binary = Get-Item -LiteralPath $Executable
$signature = Get-AuthenticodeSignature -LiteralPath $binary.FullName
if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
    throw 'The portable executable must have valid timestamped Authenticode signing.'
}
$tag = 'v' + $Version.TrimStart('v')
$name = "agora-desktop-$tag-windows-x86_64-portable"
$output = New-Item -ItemType Directory -Force -Path $OutputDirectory
$stage = Join-Path $output.FullName ($name + '-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath $binary.FullName -Destination (Join-Path $stage 'Agora Launcher.exe')
# ASCII avoids a BOM being interpreted as part of the portable data path.
Set-Content -LiteralPath (Join-Path $stage 'portable.txt') -Value 'data' -Encoding ascii
@'
Agora Launcher - Windows portable

Extract the entire ZIP to a writable folder, then run Agora Launcher.exe.
No installer is needed. Microsoft Edge WebView2 Runtime must be installed.
Agora keeps application data and browser preferences in the adjacent data folder.
AGORA_DATA_DIR overrides this location if you have set that environment variable.
Existing installed data is not automatically imported. OS credential-store sign-ins
remain tied to your Windows account; you may need to sign in on another computer.

To update, close Agora and replace Agora Launcher.exe from the new portable ZIP.
Keep portable.txt and your data folder. Do not run the MSI/EXE installer to update
this portable copy. Back up your data before moving or updating the folder.
'@ | Set-Content -LiteralPath (Join-Path $stage 'README.txt') -Encoding ascii
$archive = Join-Path $output.FullName "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$archive.sha256" -Value "$hash  $name.zip" -Encoding ascii
Write-Output $archive
