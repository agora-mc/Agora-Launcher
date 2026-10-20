param(
    [Parameter(Mandatory = $true)][string]$Executable,
    # The agora CLI from the same build. Mandatory so a release cannot quietly
    # ship a portable copy without it.
    [Parameter(Mandatory = $true)][string]$Cli,
    # The VFS DLLs and helpers staged by scripts/stage_vfs_files.py: agora_vfs.dll and its helper
    # beside the programs, and the 32-bit pair in x86\. Mandatory so a portable copy cannot be
    # built without the files that start games under the VFS.
    [Parameter(Mandatory = $true)][string]$VfsDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^v?\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$') {
    throw "Invalid release version: $Version"
}
$binary = Get-Item -LiteralPath $Executable
$cliBinary = Get-Item -LiteralPath $Cli
$vfsRelative = @('agora_vfs.dll', 'agora-vfs-helper.exe', 'x86\agora_vfs.dll', 'x86\agora-vfs-helper.exe')
$vfsFiles = @(foreach ($relative in $vfsRelative) {
    $path = Join-Path $VfsDirectory $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "The portable ZIP needs the VFS file $path. Run scripts/stage_vfs_files.py first."
    }
    Get-Item -LiteralPath $path
})
foreach ($file in @($binary, $cliBinary) + $vfsFiles) {
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
        throw "The portable executable must have valid timestamped Authenticode signing. Status: $($signature.Status); timestamp present: $([bool]$signature.TimeStamperCertificate); file: $($file.FullName)"
    }
}
$tag = 'v' + $Version.TrimStart('v')
$name = "agora-desktop-$tag-windows-x64-portable"
$output = New-Item -ItemType Directory -Force -Path $OutputDirectory
$stage = Join-Path $output.FullName ($name + '-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath $binary.FullName -Destination (Join-Path $stage 'Agora Launcher.exe')
# Beside portable.txt, so the CLI resolves the same data folder as the app.
Copy-Item -LiteralPath $cliBinary.FullName -Destination (Join-Path $stage 'agora.exe')
# Where Agora looks for them: beside the executable, and the 32-bit pair in x86\.
foreach ($relative in $vfsRelative) {
    $destination = Join-Path $stage $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath (Join-Path $VfsDirectory $relative) -Destination $destination
}
# ASCII avoids a BOM being interpreted as part of the portable data path.
Set-Content -LiteralPath (Join-Path $stage 'portable.txt') -Value 'data' -Encoding ascii
@'
Agora Launcher - Windows portable

Extract the entire ZIP to a writable folder, then run Agora Launcher.exe.
No installer is needed. Microsoft Edge WebView2 Runtime must be installed.
Agora keeps application data and browser preferences in the adjacent data folder.
AGORA_DATA_DIR overrides this location if you have set that environment variable.
Existing installed data is not automatically imported. Sign-ins are encrypted in the
data folder rather than the Windows credential store, so they travel with this copy.
Anyone who can read the data folder can read them; keep the drive safe.

agora.exe is Agora's command-line tool, for scripts and AI assistant apps. It uses
the same data folder. Settings > Services > AI assistant has a message to copy.

To update, close Agora and replace Agora Launcher.exe, agora.exe, agora_vfs.dll,
agora-vfs-helper.exe and the x86 folder from the new portable ZIP.
Keep portable.txt and your data folder. Do not run the MSI/EXE installer to update
this portable copy. Back up your data before moving or updating the folder.
'@ | Set-Content -LiteralPath (Join-Path $stage 'README.txt') -Encoding ascii
$archive = Join-Path $output.FullName "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$archive.sha256" -Value "$hash  $name.zip" -Encoding ascii
Write-Output $archive
