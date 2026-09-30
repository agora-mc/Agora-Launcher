<#
.SYNOPSIS
    Agora multi-game spike: measures, on a real Windows machine, the facts the multi-game design
    depends on. Nothing here is product code.

.DESCRIPTION
    Modes (see README.md next to this script for what each one answers and what it touches):

      Inventory   (default) Read-only. Stores, installed games, engine and mod-loader fingerprints,
                  volumes, MO2 / Vortex setups, Skyrim runtime health, Paradox mod folders.
      StockRoot   Builds a private copy of Skyrim ("stock game") next to the real install using
                  hardlinks, launches it (through SKSE when present) and records what happened.
      ToolRecord  Snapshots the game folders, waits while you run a tool (Nemesis, Pandora, ...),
                  then reports exactly what the tool wrote and how it wrote it.
      LinkArm     Hardlinks every file of a Steam game into a side folder and records identities.
      LinkCheck   After a Steam update, reports whether Steam replaced files or patched them in place.
      VerifyTest  LinkArm + corrupts one byte of one file + Steam "Verify integrity" + LinkCheck.
      Cleanup     Removes every folder this script created (only folders carrying its marker file).

    Reports are written to %LOCALAPPDATA%\AgoraSpike\reports. User-profile paths and the computer
    name are redacted unless -NoRedact is given.

.EXAMPLE
    .\agora-game-spike.ps1
.EXAMPLE
    .\agora-game-spike.ps1 -Mode StockRoot -Game SkyrimGOG
.EXAMPLE
    .\agora-game-spike.ps1 -Mode VerifyTest -SteamAppId 2379780
#>
[CmdletBinding()]
param(
    [ValidateSet('Inventory', 'StockRoot', 'ToolRecord', 'LinkArm', 'LinkCheck', 'VerifyTest', 'Cleanup')]
    [string]$Mode = 'Inventory',

    # Which Skyrim install StockRoot and ToolRecord use. Defaults: StockRoot prefers GOG, ToolRecord prefers Steam.
    [ValidateSet('SkyrimSteam', 'SkyrimGOG')]
    [string]$Game,

    # Steam app id for LinkArm / LinkCheck / VerifyTest.
    [int]$SteamAppId,

    # ToolRecord: folders to watch instead of the Skyrim defaults.
    [string[]]$Root,

    # StockRoot: copy every file instead of hardlinking (slow, uses the game's full size in disk).
    [switch]$Copy,

    # StockRoot: leave SKSE out and launch SkyrimSE.exe directly.
    [switch]$NoSkse,

    # StockRoot (Steam only): write steam_appid.txt into the stock root before launching.
    [switch]$AddSteamAppId,

    # Keep user names and paths in the report.
    [switch]$NoRedact
)

$ErrorActionPreference = 'Stop'
$SpikeVersion = 1
$SpikeHome = Join-Path $env:LOCALAPPDATA 'AgoraSpike'
$MarkerName = '.agora-spike-root'
$SkyrimSteamAppId = '489830'

# --------------------------------------------------------------------------------------------
# Native helpers: file identity (volume + file index), link count, hardlink creation.
# --------------------------------------------------------------------------------------------

$NativeSource = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public sealed class AgoraSpikeFileIdentity
{
    public string Id { get; set; }
    public uint Links { get; set; }
}

public static class AgoraSpikeNative
{
    [StructLayout(LayoutKind.Sequential)]
    private struct ByHandleFileInformation
    {
        public uint FileAttributes;
        public uint CreationTimeLow;
        public uint CreationTimeHigh;
        public uint LastAccessTimeLow;
        public uint LastAccessTimeHigh;
        public uint LastWriteTimeLow;
        public uint LastWriteTimeHigh;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security,
        uint disposition, uint flags, IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(SafeFileHandle handle, out ByHandleFileInformation info);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool CreateHardLinkW(string link, string existing, IntPtr security);

    public static AgoraSpikeFileIdentity Identify(string path)
    {
        // FILE_READ_ATTRIBUTES, share read|write|delete, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS
        using (SafeFileHandle handle = CreateFileW(path, 0x80, 7, IntPtr.Zero, 3, 0x02000000, IntPtr.Zero))
        {
            if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), path);
            ByHandleFileInformation info;
            if (!GetFileInformationByHandle(handle, out info)) throw new Win32Exception(Marshal.GetLastWin32Error(), path);
            AgoraSpikeFileIdentity identity = new AgoraSpikeFileIdentity();
            identity.Id = string.Format("{0:x8}-{1:x8}{2:x8}", info.VolumeSerialNumber, info.FileIndexHigh, info.FileIndexLow);
            identity.Links = info.NumberOfLinks;
            return identity;
        }
    }

    public static void HardLink(string link, string existing)
    {
        if (!CreateHardLinkW(link, existing, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error(), link);
    }
}
'@

$script:NativeReady = $null
function Initialize-Native {
    if ($null -ne $script:NativeReady) { return $script:NativeReady }
    try {
        if (-not ('AgoraSpikeNative' -as [type])) { Add-Type -TypeDefinition $NativeSource -Language CSharp }
        $script:NativeReady = $true
    } catch {
        Write-Warning "File identity helpers unavailable: $($_.Exception.Message)"
        $script:NativeReady = $false
    }
    return $script:NativeReady
}

function Get-FileIdentity([string]$Path) {
    if (-not (Initialize-Native)) { return $null }
    try { return [AgoraSpikeNative]::Identify($Path) } catch { return $null }
}

# --------------------------------------------------------------------------------------------
# General helpers
# --------------------------------------------------------------------------------------------

function Join-Parts {
    $result = $args[0]
    for ($i = 1; $i -lt $args.Count; $i++) { $result = [IO.Path]::Combine($result, $args[$i]) }
    return $result
}

function Get-RelativePath([string]$Base, [string]$Full) {
    $trimmed = $Base.TrimEnd('\', '/')
    if ($Full.Length -le $trimmed.Length) { return '' }
    return $Full.Substring($trimmed.Length).TrimStart('\', '/')
}

function Get-FileVersionString([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return $null }
    try {
        $info = (Get-Item -LiteralPath $Path).VersionInfo
        if ($info.FileVersion) { return ($info.FileVersion -replace ',\s*', '.' -replace '\s', '') }
        return ('{0}.{1}.{2}.{3}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart)
    } catch { return $null }
}

function Invoke-Section([string]$Name, [scriptblock]$Body) {
    Write-Host "  - $Name" -ForegroundColor DarkGray
    try { return (& $Body) }
    catch {
        Write-Warning "$Name failed: $($_.Exception.Message)"
        return [ordered]@{ error = $_.Exception.Message }
    }
}

function Protect-Text([string]$Text) {
    if ($NoRedact) { return $Text }
    # JSON-escaped and plain forms of C:\Users\<name>
    $Text = [regex]::Replace($Text, '(?i)([A-Z]:\\\\Users\\\\)[^\\"]+', '$1<user>')
    $Text = [regex]::Replace($Text, '(?i)([A-Z]:\\Users\\)[^\\"\r\n]+', '$1<user>')
    if ($env:COMPUTERNAME) {
        $Text = [regex]::Replace($Text, '(?i)\b' + [regex]::Escape($env:COMPUTERNAME) + '\b', '<computer>')
    }
    return $Text
}

function Save-Report([string]$Name, $Data) {
    $dir = Join-Path $SpikeHome 'reports'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $json = Protect-Text ($Data | ConvertTo-Json -Depth 12)
    $path = Join-Path $dir ('{0}-{1}.json' -f $Name, (Get-Date -Format 'yyyyMMdd-HHmmss'))
    [IO.File]::WriteAllText($path, $json, (New-Object Text.UTF8Encoding($false)))
    Write-Host ''
    Write-Host "Report written to: $path" -ForegroundColor Green
    Write-Host 'Send that file back to Claude (drag it into the chat, or paste its contents).' -ForegroundColor Green
    return $path
}

function Confirm-Yes([string]$Message) {
    Write-Host ''
    Write-Host $Message -ForegroundColor Yellow
    $answer = Read-Host 'Type YES to continue'
    if ($answer -cne 'YES') { throw 'Cancelled.' }
}

function New-MarkedFolder([string]$Path, $Metadata) {
    if (Test-Path -LiteralPath $Path) {
        if (-not (Test-Path -LiteralPath (Join-Path $Path $MarkerName))) {
            throw "$Path exists and was not created by this script; refusing to touch it."
        }
        Write-Host "Removing previous spike folder $Path"
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    $json = $Metadata | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText((Join-Path $Path $MarkerName), $json, (New-Object Text.UTF8Encoding($false)))
}

function Get-DocumentsPath { return [Environment]::GetFolderPath('MyDocuments') }

# --------------------------------------------------------------------------------------------
# Parsers (kept free of Windows-only calls so they can be exercised anywhere)
# --------------------------------------------------------------------------------------------

function Read-VdfValues([string]$Path) {
    # Flat read of "key" "value" pairs. The first occurrence wins, which for the keys read here
    # is the top-level value (nested sections such as InstalledDepots come later in the file).
    $values = @{}
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        if ($line -match '^\s*"([^"]+)"\s+"((?:[^"\\]|\\.)*)"') {
            $key = $Matches[1]
            if (-not $values.ContainsKey($key)) { $values[$key] = ($Matches[2] -replace '\\\\', '\') }
        }
    }
    return $values
}

function Read-VdfLibraryPaths([string]$Path) {
    $paths = @()
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        if ($line -match '^\s*"path"\s+"(.+)"') { $paths += ($Matches[1] -replace '\\\\', '\') }
    }
    return $paths
}

function Read-IniValues([string]$Path) {
    # Returns keys as "section/key". Good enough for MO2's ModOrganizer.ini, meta.ini and settings.ini.
    $values = @{}
    $section = ''
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        $t = $line.Trim()
        if ($t -eq '' -or $t.StartsWith(';') -or $t.StartsWith('#')) { continue }
        if ($t -match '^\[(.+)\]$') { $section = $Matches[1]; continue }
        $eq = $t.IndexOf('=')
        if ($eq -gt 0) { $values["$section/$($t.Substring(0, $eq).Trim())"] = $t.Substring($eq + 1).Trim() }
    }
    return $values
}

function ConvertFrom-Mo2Path([string]$Value) {
    if (-not $Value) { return $null }
    $v = $Value
    if ($v -match '^@ByteArray\((.*)\)$') { $v = $Matches[1] }
    return ($v -replace '\\\\', '\' -replace '/', '\')
}

function Get-VortexDeployments([string[]]$Dirs) {
    $out = @()
    foreach ($d in $Dirs) {
        if (-not $d -or -not (Test-Path -LiteralPath $d)) { continue }
        foreach ($f in @(Get-ChildItem -LiteralPath $d -Filter 'vortex.deployment*.json' -File -Force -ErrorAction SilentlyContinue)) {
            try {
                $j = Get-Content -LiteralPath $f.FullName -Raw | ConvertFrom-Json
                $files = @($j.files)
                $target = $j.targetPath
                if (-not $target) { $target = $d }
                $out += [pscustomobject]@{
                    manifest         = $f.FullName
                    deploymentMethod = $j.deploymentMethod
                    gameId           = $j.gameId
                    stagingPath      = $j.stagingPath
                    targetPath       = $target
                    version          = $j.version
                    fileCount        = $files.Count
                    modCount         = @($files | ForEach-Object { $_.source } | Sort-Object -Unique).Count
                    targets          = @($files | ForEach-Object { [IO.Path]::Combine($target, ([string]$_.relPath -replace '/', '\')) })
                }
            } catch {
                $out += [pscustomobject]@{ manifest = $f.FullName; error = $_.Exception.Message; targets = @() }
            }
        }
    }
    return $out
}

function Get-EngineFingerprint([string]$Dir) {
    $fp = [ordered]@{ engine = @(); loaders = @(); modFolders = @(); notes = @() }
    if (-not $Dir -or -not (Test-Path -LiteralPath $Dir)) { $fp.notes += 'install folder missing'; return $fp }
    $top = @(Get-ChildItem -LiteralPath $Dir -Force -ErrorAction SilentlyContinue)
    $topDirs = @($top | Where-Object { $_.PSIsContainer })
    $topFiles = @($top | Where-Object { -not $_.PSIsContainer })
    $has = { param($rel) Test-Path -LiteralPath (Join-Path $Dir $rel) }

    # Unity
    $unityData = @($topDirs | Where-Object { $_.Name -like '*_Data' -and ((Test-Path -LiteralPath (Join-Path $_.FullName 'globalgamemanagers')) -or (Test-Path -LiteralPath (Join-Path $_.FullName 'data.unity3d'))) })
    if ((& $has 'UnityPlayer.dll') -or $unityData.Count -gt 0) {
        if (& $has 'GameAssembly.dll') { $fp.engine += 'unity-il2cpp' } else { $fp.engine += 'unity-mono' }
    }
    # Unreal: <Project>\Content\Paks next to Engine\
    foreach ($d in $topDirs) {
        $paks = Join-Parts $d.FullName 'Content' 'Paks'
        if (Test-Path -LiteralPath $paks) {
            $fp.engine += 'unreal'
            $fp.notes += "unreal project folder: $($d.Name)"
            if (Test-Path -LiteralPath (Join-Path $paks '~mods')) { $fp.loaders += 'pak-mods(~mods)' }
            if (Test-Path -LiteralPath (Join-Path $paks 'LogicMods')) { $fp.loaders += 'ue4ss-logicmods' }
            $win64 = Join-Parts $d.FullName 'Binaries' 'Win64'
            if ((Test-Path -LiteralPath (Join-Path $win64 'ue4ss')) -or (Test-Path -LiteralPath (Join-Path $win64 'UE4SS.dll'))) { $fp.loaders += 'ue4ss' }
            if (Test-Path -LiteralPath (Join-Parts $d.FullName 'Mods' 'SML')) { $fp.loaders += 'satisfactory-mod-loader' }
            if (Test-Path -LiteralPath (Join-Path $d.FullName 'Mods')) { $fp.modFolders += "$($d.Name)\Mods" }
        }
    }
    if (@($topFiles | Where-Object { $_.Extension -eq '.pck' }).Count -gt 0) { $fp.engine += 'godot' }
    if ((& $has 'Data') -and @(Get-ChildItem -LiteralPath (Join-Path $Dir 'Data') -Filter '*.esm' -File -ErrorAction SilentlyContinue | Select-Object -First 1).Count -gt 0) {
        $fp.engine += 'creation'
        $fp.modFolders += 'Data'
    }
    if (& $has 'archive\pc\content') { $fp.engine += 'redengine4' }
    if (& $has 'content\content0') { $fp.engine += 'redengine3' }
    if ((& $has 'launcher-settings.json') -or (& $has 'launcher\launcher-settings.json')) { $fp.engine += 'paradox' }
    if (& $has 'love.dll') { $fp.engine += 'love2d' }
    if ((& $has 'jre') -or @($topFiles | Where-Object { $_.Extension -eq '.jar' }).Count -gt 0) { $fp.engine += 'java' }
    if (& $has 'data\base\info.json') { $fp.engine += 'factorio' }
    if (& $has 'Modules\Native') { $fp.engine += 'bannerlord'; $fp.modFolders += 'Modules' }
    if (& $has 'GameData\Squad') { $fp.modFolders += 'GameData' }

    # Loaders and injectors present in the install
    if (& $has 'BepInEx') { $fp.loaders += 'bepinex' }
    if (& $has 'doorstop_config.ini') { $fp.loaders += 'unity-doorstop' }
    if (& $has 'MelonLoader') { $fp.loaders += 'melonloader' }
    if (& $has 'skse64_loader.exe') { $fp.loaders += 'skse64' }
    if (& $has 'f4se_loader.exe') { $fp.loaders += 'f4se' }
    if (& $has 'red4ext') { $fp.loaders += 'red4ext' }
    if (& $has 'bin\x64\plugins\cyber_engine_tweaks') { $fp.loaders += 'cyber-engine-tweaks' }
    if (& $has 'r6\scripts') { $fp.loaders += 'redscript-folder' }
    if (& $has 'ModTheSpire.jar') { $fp.loaders += 'modthespire' }
    foreach ($proxy in 'winhttp.dll', 'version.dll', 'dinput8.dll', 'dxgi.dll', 'd3d11.dll', 'd3d9.dll') {
        if (& $has $proxy) { $fp.loaders += "proxy-dll:$proxy" }
    }
    foreach ($folder in 'Mods', 'mods', 'BepInEx\plugins', 'Plugins', 'dlc') {
        if (& $has $folder) { $fp.modFolders += $folder }
    }
    $fp.engine = @($fp.engine | Sort-Object -Unique)
    $fp.loaders = @($fp.loaders | Sort-Object -Unique)
    $fp.modFolders = @($fp.modFolders | Sort-Object -Unique)
    return $fp
}

# --------------------------------------------------------------------------------------------
# Store discovery
# --------------------------------------------------------------------------------------------

function Get-SteamRoot {
    foreach ($key in 'HKCU:\Software\Valve\Steam', 'HKLM:\SOFTWARE\WOW6432Node\Valve\Steam', 'HKLM:\SOFTWARE\Valve\Steam') {
        try {
            $p = Get-ItemProperty -Path $key -ErrorAction Stop
            foreach ($name in 'SteamPath', 'InstallPath') {
                $v = $p.$name
                if ($v) {
                    $v = $v -replace '/', '\'
                    if (Test-Path -LiteralPath $v) { return $v }
                }
            }
        } catch { }
    }
    return $null
}

$AutoUpdateLabels = @{
    '0' = 'Always keep this game updated'
    '1' = 'Only update this game when I launch it'
    '2' = 'High priority'
}

function Get-SteamApps {
    $steamRoot = Get-SteamRoot
    if (-not $steamRoot) { return @() }
    $libraries = @($steamRoot)
    $vdf = Join-Parts $steamRoot 'steamapps' 'libraryfolders.vdf'
    if (Test-Path -LiteralPath $vdf) {
        foreach ($p in (Read-VdfLibraryPaths $vdf)) { if ($libraries -notcontains $p) { $libraries += $p } }
    }
    $apps = @()
    foreach ($lib in $libraries) {
        $steamapps = Join-Path $lib 'steamapps'
        if (-not (Test-Path -LiteralPath $steamapps)) { continue }
        foreach ($acf in @(Get-ChildItem -LiteralPath $steamapps -Filter 'appmanifest_*.acf' -File -ErrorAction SilentlyContinue)) {
            try {
                $v = Read-VdfValues $acf.FullName
                $dir = Join-Parts $steamapps 'common' $v['installdir']
                $updated = $null
                if ($v['LastUpdated']) { $updated = [DateTimeOffset]::FromUnixTimeSeconds([int64]$v['LastUpdated']).UtcDateTime.ToString('o') }
                $apps += [pscustomobject]@{
                    store              = 'steam'
                    appId              = $v['appid']
                    name               = $v['name']
                    installDir         = $dir
                    installed          = (Test-Path -LiteralPath $dir)
                    buildId            = $v['buildid']
                    sizeOnDiskGB       = [math]::Round(([double]$v['SizeOnDisk']) / 1GB, 2)
                    lastUpdatedUtc     = $updated
                    autoUpdateBehavior = $AutoUpdateLabels[[string]$v['AutoUpdateBehavior']]
                    library            = $lib
                }
            } catch { Write-Warning "Could not read $($acf.Name): $($_.Exception.Message)" }
        }
    }
    return $apps
}

function Get-GogGames {
    $out = @()
    $seen = @{}
    foreach ($base in 'HKLM:\SOFTWARE\WOW6432Node\GOG.com\Games', 'HKLM:\SOFTWARE\GOG.com\Games') {
        if (-not (Test-Path $base)) { continue }
        foreach ($k in @(Get-ChildItem $base -ErrorAction SilentlyContinue)) {
            $p = Get-ItemProperty $k.PSPath -ErrorAction SilentlyContinue
            if (-not $p -or -not $p.path) { continue }
            $id = [string]$p.gameID
            if ($seen.ContainsKey($id)) { continue }
            $seen[$id] = $true
            $out += [pscustomobject]@{
                store      = 'gog'
                appId      = $id
                name       = $p.gameName
                installDir = $p.path
                installed  = (Test-Path -LiteralPath $p.path)
                version    = $p.ver
                buildId    = $p.buildId
                exe        = $p.exe
            }
        }
    }
    return $out
}

function Get-EpicGames {
    $out = @()
    $dir = Join-Parts $env:ProgramData 'Epic' 'EpicGamesLauncher' 'Data' 'Manifests'
    if (-not (Test-Path -LiteralPath $dir)) { return $out }
    foreach ($f in @(Get-ChildItem -LiteralPath $dir -Filter '*.item' -File -ErrorAction SilentlyContinue)) {
        try {
            $j = Get-Content -LiteralPath $f.FullName -Raw | ConvertFrom-Json
            $out += [pscustomobject]@{
                store      = 'epic'
                appId      = $j.AppName
                name       = $j.DisplayName
                installDir = $j.InstallLocation
                installed  = (Test-Path -LiteralPath $j.InstallLocation)
                version    = $j.AppVersionString
                exe        = $j.LaunchExecutable
            }
        } catch { }
    }
    return $out
}

function Test-Writable([string]$Dir) {
    $probe = Join-Path $Dir ('.agora-spike-probe-' + [guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::WriteAllText($probe, '')
        Remove-Item -LiteralPath $probe -Force
        return $true
    } catch { return $false }
}

function Get-XboxGames {
    $out = @()
    $roots = @()
    foreach ($drive in @(Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        if ($drive.Root -notmatch '^[A-Za-z]:\\$') { continue }
        $gamingRoot = Join-Path $drive.Root '.GamingRoot'
        $folder = Join-Path $drive.Root 'XboxGames'
        if (Test-Path -LiteralPath $folder) { $roots += $folder }
        elseif (Test-Path -LiteralPath $gamingRoot) { $roots += "(custom folder on $($drive.Root), see .GamingRoot)" }
    }
    foreach ($folder in $roots) {
        if (-not (Test-Path -LiteralPath $folder)) {
            $out += [pscustomobject]@{ store = 'xbox'; name = $folder; installed = $false }
            continue
        }
        foreach ($g in @(Get-ChildItem -LiteralPath $folder -Directory -ErrorAction SilentlyContinue)) {
            $content = Join-Path $g.FullName 'Content'
            $readable = $false
            $exes = @()
            try {
                $exes = @(Get-ChildItem -LiteralPath $content -Filter '*.exe' -File -ErrorAction Stop | ForEach-Object { $_.Name })
                $readable = $true
            } catch { }
            $out += [pscustomobject]@{
                store                = 'xbox'
                name                 = $g.Name
                installDir           = $content
                installed            = (Test-Path -LiteralPath $content)
                contentReadable      = $readable
                contentWritable      = ($readable -and (Test-Writable $content))
                microsoftGameConfig  = (Test-Path -LiteralPath (Join-Path $content 'MicrosoftGame.config'))
                topLevelExecutables  = $exes
            }
        }
    }
    # Store packages that declare themselves games but live outside XboxGames (older WindowsApps installs).
    try {
        foreach ($p in @(Get-AppxPackage -ErrorAction Stop | Where-Object { -not $_.IsFramework -and $_.SignatureKind -eq 'Store' })) {
            $loc = $p.InstallLocation
            if (-not $loc -or $loc -match '\\XboxGames\\') { continue }
            $isGame = $false
            try { $isGame = Test-Path -LiteralPath (Join-Path $loc 'MicrosoftGame.config') } catch { }
            if (-not $isGame) { continue }
            $out += [pscustomobject]@{
                store           = 'xbox-windowsapps'
                name            = $p.Name
                version         = [string]$p.Version
                installDir      = $loc
                installed       = $true
                contentReadable = $true
                contentWritable = (Test-Writable $loc)
            }
        }
    } catch { }
    return $out
}

# --------------------------------------------------------------------------------------------
# Mod managers
# --------------------------------------------------------------------------------------------

function Get-NxmHandler {
    foreach ($key in 'HKCU:\Software\Classes\nxm\shell\open\command', 'HKLM:\SOFTWARE\Classes\nxm\shell\open\command') {
        if (Test-Path $key) {
            try { return [pscustomobject]@{ key = $key; command = (Get-Item $key).GetValue('') } } catch { }
        }
    }
    return $null
}

function Get-Mo2Instance([string]$InstanceDir, [string]$Kind) {
    $iniPath = Join-Path $InstanceDir 'ModOrganizer.ini'
    $ini = Read-IniValues $iniPath
    $base = ConvertFrom-Mo2Path $ini['Settings/base_directory']
    if (-not $base) { $base = $InstanceDir }
    $resolve = {
        param($key, $default)
        $v = ConvertFrom-Mo2Path $ini["Settings/$key"]
        if (-not $v) { return (Join-Path $base $default) }
        return ($v -replace '%BASE_DIR%', $base)
    }
    $modsDir = & $resolve 'mod_directory' 'mods'
    $profilesDir = & $resolve 'profiles_directory' 'profiles'
    $overwriteDir = & $resolve 'overwrite_directory' 'overwrite'

    $mods = @()
    if (Test-Path -LiteralPath $modsDir) { $mods = @(Get-ChildItem -LiteralPath $modsDir -Directory -ErrorAction SilentlyContinue) }
    $nexus = 0; $withArchive = 0; $separators = 0
    $addressLibraries = @()
    foreach ($m in $mods) {
        if ($m.Name -like '*_separator') { $separators++; continue }
        $meta = Join-Path $m.FullName 'meta.ini'
        if (Test-Path -LiteralPath $meta) {
            try {
                $mv = Read-IniValues $meta
                $id = 0
                if ([int]::TryParse([string]$mv['General/modid'], [ref]$id) -and $id -gt 0) { $nexus++ }
                if ($mv['General/installationFile']) { $withArchive++ }
            } catch { }
        }
        $plugins = Join-Parts $m.FullName 'SKSE' 'Plugins'
        if (Test-Path -LiteralPath $plugins) {
            $addressLibraries += @(Get-ChildItem -LiteralPath $plugins -Filter 'versionlib-*.bin' -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
        }
    }

    $profiles = @()
    if (Test-Path -LiteralPath $profilesDir) {
        foreach ($p in @(Get-ChildItem -LiteralPath $profilesDir -Directory -ErrorAction SilentlyContinue)) {
            $modlist = Join-Path $p.FullName 'modlist.txt'
            $lines = @()
            if (Test-Path -LiteralPath $modlist) { $lines = @([IO.File]::ReadAllLines($modlist)) }
            $pluginsTxt = Join-Path $p.FullName 'plugins.txt'
            $pluginLines = @()
            if (Test-Path -LiteralPath $pluginsTxt) { $pluginLines = @([IO.File]::ReadAllLines($pluginsTxt) | Where-Object { $_ -and -not $_.StartsWith('#') }) }
            $settings = @{}
            $settingsPath = Join-Path $p.FullName 'settings.ini'
            if (Test-Path -LiteralPath $settingsPath) { $settings = Read-IniValues $settingsPath }
            $profiles += [pscustomobject]@{
                name             = $p.Name
                modsEnabled      = @($lines | Where-Object { $_.StartsWith('+') -and $_ -notlike '*_separator' }).Count
                modsDisabled     = @($lines | Where-Object { $_.StartsWith('-') -and $_ -notlike '*_separator' }).Count
                unmanaged        = @($lines | Where-Object { $_.StartsWith('*') }).Count
                pluginsListed    = $pluginLines.Count
                pluginsEnabled   = @($pluginLines | Where-Object { $_.StartsWith('*') }).Count
                localSaves       = $settings['General/LocalSaves']
                localSettings    = $settings['General/LocalSettings']
            }
        }
    }
    $overwriteFiles = 0
    if (Test-Path -LiteralPath $overwriteDir) { $overwriteFiles = @(Get-ChildItem -LiteralPath $overwriteDir -Recurse -File -Force -ErrorAction SilentlyContinue).Count }

    return [pscustomobject]@{
        kind             = $Kind
        instanceDir      = $InstanceDir
        gameName         = $ini['General/gameName']
        gameEdition      = $ini['General/game_edition']
        gamePath         = ConvertFrom-Mo2Path $ini['General/gamePath']
        selectedProfile  = ConvertFrom-Mo2Path $ini['General/selected_profile']
        mo2Version       = $ini['General/version']
        modCount         = $mods.Count - $separators
        separators       = $separators
        nexusLinkedMods  = $nexus
        modsWithArchive  = $withArchive
        addressLibraries = @($addressLibraries | Sort-Object -Unique)
        overwriteFiles   = $overwriteFiles
        profiles         = $profiles
    }
}

function Get-Mo2Setups {
    $instances = @()
    $installs = @()
    $global = Join-Path $env:LOCALAPPDATA 'ModOrganizer'
    if (Test-Path -LiteralPath $global) {
        foreach ($d in @(Get-ChildItem -LiteralPath $global -Directory -ErrorAction SilentlyContinue)) {
            if (Test-Path -LiteralPath (Join-Path $d.FullName 'ModOrganizer.ini')) {
                try { $instances += Get-Mo2Instance $d.FullName 'global' }
                catch { $instances += [pscustomobject]@{ instanceDir = $d.FullName; error = $_.Exception.Message } }
            }
        }
    }
    # MO2 installs: the nxm handler usually points at one; portable installs are their own instance.
    $candidates = @()
    $nxm = Get-NxmHandler
    if ($nxm -and $nxm.command -match '"?([^"]*?)\\nxmhandler\.exe') { $candidates += $Matches[1] }
    foreach ($c in @("$env:ProgramFiles\Mod Organizer 2", "$env:LOCALAPPDATA\Programs\Mod Organizer 2", 'C:\Modding\MO2', 'C:\MO2')) { $candidates += $c }
    foreach ($c in ($candidates | Sort-Object -Unique)) {
        $exe = Join-Path $c 'ModOrganizer.exe'
        if (-not (Test-Path -LiteralPath $exe)) { continue }
        $installs += [pscustomobject]@{
            dir      = $c
            version  = Get-FileVersionString $exe
            portable = (Test-Path -LiteralPath (Join-Path $c 'portable.txt'))
            usvfs    = @(Get-ChildItem -LiteralPath $c -Filter 'usvfs*' -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
        }
        if ((Test-Path -LiteralPath (Join-Path $c 'portable.txt')) -and (Test-Path -LiteralPath (Join-Path $c 'ModOrganizer.ini'))) {
            try { $instances += Get-Mo2Instance $c 'portable' } catch { }
        }
    }
    return [ordered]@{ installs = $installs; instances = $instances }
}

function Get-VortexSetup {
    $root = Join-Path $env:APPDATA 'Vortex'
    $exe = Join-Parts $env:ProgramFiles 'Black Tree Gaming Ltd' 'Vortex' 'Vortex.exe'
    $result = [ordered]@{
        installed  = (Test-Path -LiteralPath $exe)
        version    = Get-FileVersionString $exe
        dataFolder = (Test-Path -LiteralPath $root)
        stateDbMB  = $null
        staging    = @()
    }
    if (Test-Path -LiteralPath $root) {
        $state = Join-Path $root 'state.v2'
        if (Test-Path -LiteralPath $state) {
            $result.stateDbMB = [math]::Round((@(Get-ChildItem -LiteralPath $state -Recurse -File -ErrorAction SilentlyContinue) | Measure-Object Length -Sum).Sum / 1MB, 1)
        }
        foreach ($d in @(Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue)) {
            $mods = Join-Path $d.FullName 'mods'
            if (Test-Path -LiteralPath $mods) {
                $result.staging += [pscustomobject]@{
                    gameId   = $d.Name
                    modCount = @(Get-ChildItem -LiteralPath $mods -Directory -ErrorAction SilentlyContinue).Count
                }
            }
        }
    }
    return $result
}

# --------------------------------------------------------------------------------------------
# Skyrim
# --------------------------------------------------------------------------------------------

function Get-SkyrimInstalls {
    $list = @()
    foreach ($a in @(Get-SteamApps)) {
        if ($a.appId -eq $SkyrimSteamAppId -and $a.installed) {
            $list += [pscustomobject]@{ flavor = 'Steam'; key = 'SkyrimSteam'; installDir = $a.installDir; appDataName = 'Skyrim Special Edition'; steam = $a }
        }
    }
    foreach ($g in @(Get-GogGames)) {
        if ($g.name -match 'Skyrim' -and $g.installed -and (Test-Path -LiteralPath (Join-Path $g.installDir 'SkyrimSE.exe'))) {
            $list += [pscustomobject]@{ flavor = 'GOG'; key = 'SkyrimGOG'; installDir = $g.installDir; appDataName = 'Skyrim Special Edition GOG'; steam = $null }
        }
    }
    return $list
}

function Get-SkyrimDetails($Install) {
    $dir = $Install.installDir
    $exeVersion = Get-FileVersionString (Join-Path $dir 'SkyrimSE.exe')
    $triple = $null
    if ($exeVersion) { $triple = ($exeVersion.Split('.')[0..2] -join '.') }

    $skseDlls = @(Get-ChildItem -LiteralPath $dir -Filter 'skse64_1_*.dll' -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
    $skseTargets = @($skseDlls | ForEach-Object { $_ -replace '^skse64_(\d+)_(\d+)_(\d+)\.dll$', '$1.$2.$3' })
    $data = Join-Path $dir 'Data'
    $pluginsDir = Join-Parts $data 'SKSE' 'Plugins'
    $addressLibs = @(Get-ChildItem -LiteralPath $pluginsDir -Filter 'versionlib-*.bin' -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
    $expectedAddressLib = $null
    if ($exeVersion) { $expectedAddressLib = 'versionlib-' + ($exeVersion -replace '\.', '-') + '.bin' }

    $dataFiles = @(Get-ChildItem -LiteralPath $data -Recurse -File -Force -ErrorAction SilentlyContinue)
    $linked = 0
    if (Initialize-Native) {
        foreach ($f in $dataFiles) {
            $id = Get-FileIdentity $f.FullName
            if ($id -and $id.Links -gt 1) { $linked++ }
        }
    }
    $topPlugins = @($dataFiles | Where-Object { $_.DirectoryName -eq $data -and $_.Extension -in '.esp', '.esm', '.esl' })

    $appData = Join-Path $env:LOCALAPPDATA $Install.appDataName
    $pluginsTxt = Join-Path $appData 'plugins.txt'
    $pluginLines = @()
    if (Test-Path -LiteralPath $pluginsTxt) { $pluginLines = @([IO.File]::ReadAllLines($pluginsTxt) | Where-Object { $_ -and -not $_.StartsWith('#') }) }
    $myGames = Join-Parts (Get-DocumentsPath) 'My Games' $Install.appDataName
    $skseLog = Join-Parts $myGames 'SKSE' 'skse64.log'

    $vortex = @(Get-VortexDeployments @($dir, $data))
    return [pscustomobject]@{
        flavor                   = $Install.flavor
        installDir               = $dir
        drive                    = [IO.Path]::GetPathRoot($dir)
        exeVersion               = $exeVersion
        steamBuildId             = $(if ($Install.steam) { $Install.steam.buildId } else { $null })
        steamAutoUpdate          = $(if ($Install.steam) { $Install.steam.autoUpdateBehavior } else { $null })
        skseLoaderPresent        = (Test-Path -LiteralPath (Join-Path $dir 'skse64_loader.exe'))
        skseLoaderVersion        = Get-FileVersionString (Join-Path $dir 'skse64_loader.exe')
        skseRuntimeTargets       = $skseTargets
        skseMatchesGame          = ($triple -and ($skseTargets -contains $triple))
        addressLibrariesInData   = $addressLibs
        expectedAddressLibrary   = $expectedAddressLib
        addressLibraryMatches    = ($expectedAddressLib -and ($addressLibs -contains $expectedAddressLib))
        nemesisEngineInData      = (Test-Path -LiteralPath (Join-Path $data 'Nemesis_Engine'))
        pandoraEngineInData      = (Test-Path -LiteralPath (Join-Path $data 'Pandora_Engine'))
        dataFileCount            = $dataFiles.Count
        dataFilesHardlinked      = $linked
        dataTopLevelPlugins      = $topPlugins.Count
        creationClubPlugins      = @($topPlugins | Where-Object { $_.Name -like 'cc*' }).Count
        pluginsTxtEntries        = $pluginLines.Count
        pluginsTxtEnabled        = @($pluginLines | Where-Object { $_.StartsWith('*') }).Count
        myGamesExists            = (Test-Path -LiteralPath $myGames)
        saveCount                = @(Get-ChildItem -LiteralPath (Join-Path $myGames 'Saves') -Filter '*.ess' -File -ErrorAction SilentlyContinue).Count
        skseLogLastWriteUtc      = $(if (Test-Path -LiteralPath $skseLog) { (Get-Item -LiteralPath $skseLog).LastWriteTimeUtc.ToString('o') } else { $null })
        vortexDeployments        = @($vortex | Select-Object manifest, deploymentMethod, gameId, stagingPath, targetPath, version, fileCount, modCount, error)
    }
}

# --------------------------------------------------------------------------------------------
# Mode: Inventory
# --------------------------------------------------------------------------------------------

function Get-SystemInfo {
    $os = $null
    try { $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop } catch { }
    $isAdmin = $false
    try {
        $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch { }
    $devMode = $null
    try { $devMode = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' -ErrorAction Stop).AllowDevelopmentWithoutDevLicense } catch { }
    $longPaths = $null
    try { $longPaths = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction Stop).LongPathsEnabled } catch { }
    $av = @()
    try { $av = @(Get-CimInstance -Namespace 'root/SecurityCenter2' -ClassName AntiVirusProduct -ErrorAction Stop | ForEach-Object { $_.displayName }) } catch { }
    $projfs = 'unknown (needs an elevated shell to query)'
    if ($isAdmin) {
        try { $projfs = [string](Get-WindowsOptionalFeature -Online -FeatureName Client-ProjFS -ErrorAction Stop).State } catch { }
    }
    $volumes = @()
    try {
        $volumes = @(Get-Volume -ErrorAction Stop | Where-Object { $_.DriveLetter } | Sort-Object DriveLetter | ForEach-Object {
                [pscustomobject]@{
                    drive      = "$($_.DriveLetter):\"
                    fileSystem = $_.FileSystemType
                    type       = [string]$_.DriveType
                    sizeGB     = [math]::Round($_.Size / 1GB, 0)
                    freeGB     = [math]::Round($_.SizeRemaining / 1GB, 0)
                }
            })
    } catch { }
    return [ordered]@{
        os                = $(if ($os) { "$($os.Caption) $($os.Version)" } else { $null })
        powershell        = $PSVersionTable.PSVersion.ToString()
        elevated          = $isAdmin
        developerMode     = $devMode
        longPathsEnabled  = $longPaths
        antivirus         = $av
        projectedFsFeature = $projfs
        documentsFolder   = Get-DocumentsPath
        localAppData      = $env:LOCALAPPDATA
        volumes           = $volumes
    }
}

function Get-ParadoxData {
    $root = Join-Path (Get-DocumentsPath) 'Paradox Interactive'
    $games = @()
    if (Test-Path -LiteralPath $root) {
        foreach ($g in @(Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue)) {
            $modDir = Join-Path $g.FullName 'mod'
            $games += [pscustomobject]@{
                game           = $g.Name
                userDir        = $g.FullName
                descriptors    = @(Get-ChildItem -LiteralPath $modDir -Filter '*.mod' -File -ErrorAction SilentlyContinue).Count
                modFolders     = @(Get-ChildItem -LiteralPath $modDir -Directory -ErrorAction SilentlyContinue).Count
                dlcLoadJson    = (Test-Path -LiteralPath (Join-Path $g.FullName 'dlc_load.json'))
                contentLoadJson = (Test-Path -LiteralPath (Join-Path $g.FullName 'content_load.json'))
            }
        }
    }
    $launcher = Join-Parts $env:LOCALAPPDATA 'Paradox Interactive' 'launcher-v2'
    return [ordered]@{
        documentsRoot    = $root
        games            = $games
        launcherV2Folder = (Test-Path -LiteralPath $launcher)
        launcherDatabase = @(Get-ChildItem -LiteralPath $launcher -Filter '*.sqlite' -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
    }
}

function Invoke-Inventory {
    Write-Host 'Agora game spike: inventory (read-only apart from one empty probe file per Xbox game folder)' -ForegroundColor Cyan
    $report = [ordered]@{ spikeVersion = $SpikeVersion; mode = 'Inventory'; startedUtc = (Get-Date).ToUniversalTime().ToString('o') }
    $report.system = Invoke-Section 'system and volumes' { Get-SystemInfo }
    $games = @()
    $games += @(Invoke-Section 'Steam' { Get-SteamApps })
    $games += @(Invoke-Section 'GOG' { Get-GogGames })
    $games += @(Invoke-Section 'Epic' { Get-EpicGames })
    $games += @(Invoke-Section 'Xbox / Microsoft Store' { Get-XboxGames })
    $games = @($games | Where-Object { $_ -isnot [System.Collections.IDictionary] -and $_.name -and $_.name -notmatch 'Redistributable|Proton|Steam Linux Runtime|Steamworks' })
    Write-Host '  - engine and loader fingerprints' -ForegroundColor DarkGray
    foreach ($g in $games) {
        $fp = $null
        if ($g.installed -and $g.installDir) {
            try { $fp = Get-EngineFingerprint $g.installDir } catch { $fp = [ordered]@{ error = $_.Exception.Message } }
        }
        $g | Add-Member -NotePropertyName fingerprint -NotePropertyValue $fp -Force
        $drive = $null
        if ($g.installDir -and $g.installDir -match '^[A-Za-z]:\\') { $drive = [IO.Path]::GetPathRoot($g.installDir) }
        $g | Add-Member -NotePropertyName drive -NotePropertyValue $drive -Force
    }
    $report.games = $games
    $report.modManagers = [ordered]@{
        nxmHandler = Invoke-Section 'nxm:// handler' { Get-NxmHandler }
        mo2        = Invoke-Section 'Mod Organizer 2' { Get-Mo2Setups }
        vortex     = Invoke-Section 'Vortex' { Get-VortexSetup }
    }
    $report.skyrim = @(Invoke-Section 'Skyrim (counts hardlinks in Data; can take a minute)' {
            @(Get-SkyrimInstalls | ForEach-Object { Get-SkyrimDetails $_ })
        })
    $report.paradox = Invoke-Section 'Paradox user folders' { Get-ParadoxData }
    $report.finishedUtc = (Get-Date).ToUniversalTime().ToString('o')

    Write-Host ''
    Write-Host 'Installed games' -ForegroundColor Cyan
    $games | Where-Object { $_.installed } | Sort-Object store, name | ForEach-Object {
        $engine = ''; $loaders = ''
        if ($_.fingerprint -and $_.fingerprint.engine) { $engine = ($_.fingerprint.engine -join ',') }
        if ($_.fingerprint -and $_.fingerprint.loaders) { $loaders = ($_.fingerprint.loaders -join ',') }
        '{0,-6} {1,-45} {2,-20} {3}' -f $_.store, ([string]$_.name).Substring(0, [math]::Min(45, ([string]$_.name).Length)), $engine, $loaders
    } | Write-Host
    foreach ($s in @($report.skyrim)) {
        if ($s -is [System.Collections.IDictionary]) { continue }
        Write-Host ''
        Write-Host "Skyrim ($($s.flavor)) $($s.exeVersion)" -ForegroundColor Cyan
        $mark = { param($ok) if ($ok) { '[ok]' } else { '[!!]' } }
        if ($s.skseLoaderPresent) { Write-Host ("  {0} SKSE built for {1}" -f (& $mark $s.skseMatchesGame), ($s.skseRuntimeTargets -join ', ')) }
        else { Write-Host '  [--] SKSE not in the game folder (may live in a mod manager instead)' }
        if ($s.addressLibrariesInData.Count -gt 0) { Write-Host ("  {0} Address Library: expected {1}" -f (& $mark $s.addressLibraryMatches), $s.expectedAddressLibrary) }
        if ($s.steamAutoUpdate) { Write-Host "  [..] Steam auto-update: $($s.steamAutoUpdate)" }
        Write-Host "  [..] Data: $($s.dataFileCount) files, $($s.dataFilesHardlinked) hardlinked (Vortex deploys by hardlink)"
    }
    Save-Report 'inventory' $report | Out-Null
}

# --------------------------------------------------------------------------------------------
# Mode: StockRoot
# --------------------------------------------------------------------------------------------

function Select-SkyrimInstall([string]$Preferred, [string]$Fallback) {
    $installs = @(Get-SkyrimInstalls)
    if ($installs.Count -eq 0) { throw 'No Skyrim Special/Anniversary Edition install found on Steam or GOG.' }
    $key = $Game
    if (-not $key) { $key = $Preferred }
    $hit = @($installs | Where-Object { $_.key -eq $key })
    if ($hit.Count -eq 0 -and -not $Game) { $hit = @($installs | Where-Object { $_.key -eq $Fallback }) }
    if ($hit.Count -eq 0) { throw "Skyrim install '$key' not found. Found: $(($installs | ForEach-Object { $_.key }) -join ', ')" }
    return $hit[0]
}

function Invoke-StockRoot {
    if (-not (Initialize-Native)) { throw 'StockRoot needs the native file helpers.' }
    $install = Select-SkyrimInstall 'SkyrimGOG' 'SkyrimSteam'
    $source = $install.installDir.TrimEnd('\')
    $drive = [IO.Path]::GetPathRoot($source)
    $stock = Join-Parts $drive 'AgoraSpike' ('stock-' + $install.flavor.ToLower())
    $method = 'hardlink'
    if ($Copy) { $method = 'copy' }

    Confirm-Yes (@"
StockRoot will:
  1. Build $stock from $source
     ($method for data files; executables and DLLs are always copied so the copy has its own exe).
     Mod files deployed by Vortex, SKSE plugins, ENB/ReShade injectors are left out.
  2. Back up plugins.txt / loadorder.txt and your Skyrim INI files, and restore them afterwards.
  3. Launch Skyrim from the new folder$(if (-not $NoSkse) { ' through SKSE when present' }).
You then reach the main menu, do NOT load or make a save, and quit.
Nothing in the real install is written to. The new folder costs almost no disk when hardlinked.
"@)

    New-MarkedFolder $stock ([ordered]@{ createdBy = 'agora-game-spike'; mode = 'StockRoot'; source = $source; createdUtc = (Get-Date).ToUniversalTime().ToString('o') })

    $vortexTargets = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($dep in @(Get-VortexDeployments @($source, (Join-Path $source 'Data')))) {
        foreach ($t in @($dep.targets)) { [void]$vortexTargets.Add($t) }
    }
    $injectors = @('d3d11.dll', 'dxgi.dll', 'd3d9.dll', 'dinput8.dll', 'winhttp.dll', 'version.dll', 'd3dcompiler_46e.dll', 'enblocal.ini', 'enbseries.ini')

    $stats = [ordered]@{ hardlinked = 0; copied = 0; hardlinkFailedCopied = 0; skipped = [ordered]@{} }
    $bytesLinked = 0L
    $bytesCopied = 0L
    $linkedOriginals = @()
    $created = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    foreach ($f in @(Get-ChildItem -LiteralPath $source -Recurse -File -Force -ErrorAction SilentlyContinue)) {
        $rel = Get-RelativePath $source $f.FullName
        $topLevel = ($rel -notmatch '\\')
        $isSkse = $topLevel -and $f.Name -like 'skse64_*'
        $reason = $null
        if ($isSkse -and $NoSkse) { $reason = 'skse (disabled)' }
        elseif (-not $isSkse -and $vortexTargets.Contains($f.FullName)) { $reason = 'deployed by Vortex' }
        elseif ($topLevel -and ($injectors -contains $f.Name.ToLower())) { $reason = 'graphics injector' }
        elseif ($rel -like 'Data\SKSE\Plugins\*') { $reason = 'SKSE plugin' }
        elseif ($f.Name -like 'vortex.deployment*') { $reason = 'Vortex manifest' }
        elseif ($rel -like 'enbseries\*' -or $rel -like 'reshade-shaders\*') { $reason = 'graphics injector' }
        if ($reason) {
            if (-not $stats.skipped.Contains($reason)) { $stats.skipped[$reason] = 0 }
            $stats.skipped[$reason]++
            continue
        }
        $dest = Join-Path $stock $rel
        $destDir = Split-Path -Parent $dest
        if (-not (Test-Path -LiteralPath $destDir)) { New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
        $alwaysCopy = $f.Extension -in '.exe', '.dll'
        if ($Copy -or $alwaysCopy) {
            Copy-Item -LiteralPath $f.FullName -Destination $dest -Force
            $stats.copied++; $bytesCopied += $f.Length
        } else {
            try {
                [AgoraSpikeNative]::HardLink($dest, $f.FullName)
                $stats.hardlinked++; $bytesLinked += $f.Length
                $linkedOriginals += [pscustomobject]@{ path = $f.FullName; length = $f.Length; lastWriteUtc = $f.LastWriteTimeUtc }
            } catch {
                Copy-Item -LiteralPath $f.FullName -Destination $dest -Force
                $stats.hardlinkFailedCopied++; $bytesCopied += $f.Length
            }
        }
        [void]$created.Add($dest)
    }
    $timer.Stop()
    $stats.buildSeconds = [math]::Round($timer.Elapsed.TotalSeconds, 1)
    $stats.gbHardlinked = [math]::Round($bytesLinked / 1GB, 2)
    $stats.gbCopied = [math]::Round($bytesCopied / 1GB, 2)
    Write-Host ("Built in {0}s: {1} hardlinked ({2} GB), {3} copied ({4} GB)" -f $stats.buildSeconds, $stats.hardlinked, $stats.gbHardlinked, $stats.copied, $stats.gbCopied)

    if ($AddSteamAppId -and $install.flavor -eq 'Steam') {
        [IO.File]::WriteAllText((Join-Path $stock 'steam_appid.txt'), $SkyrimSteamAppId)
        [void]$created.Add((Join-Path $stock 'steam_appid.txt'))
    }

    # Back up the per-user files the game (or the AE Creations menu) may rewrite.
    $appData = Join-Path $env:LOCALAPPDATA $install.appDataName
    $myGames = Join-Parts (Get-DocumentsPath) 'My Games' $install.appDataName
    $userFiles = @()
    foreach ($n in 'plugins.txt', 'loadorder.txt', 'DLCList.txt') { $userFiles += (Join-Path $appData $n) }
    foreach ($n in 'Skyrim.ini', 'SkyrimPrefs.ini', 'SkyrimCustom.ini') { $userFiles += (Join-Path $myGames $n) }
    $backupDir = Join-Path $SpikeHome ('backup-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
    $backups = @()
    $i = 0
    foreach ($uf in $userFiles) {
        if (-not (Test-Path -LiteralPath $uf)) { continue }
        $copyPath = Join-Path $backupDir ('{0:d2}-{1}' -f $i, (Split-Path -Leaf $uf)); $i++
        Copy-Item -LiteralPath $uf -Destination $copyPath -Force
        $backups += [pscustomobject]@{ original = $uf; backup = $copyPath; hash = (Get-FileHash -LiteralPath $uf -Algorithm SHA256).Hash }
    }
    Write-Host "Backed up $($backups.Count) user files to $backupDir"

    $useSkse = (-not $NoSkse) -and (Test-Path -LiteralPath (Join-Path $stock 'skse64_loader.exe'))
    $launchExe = Join-Path $stock 'SkyrimSE.exe'
    if ($useSkse) { $launchExe = Join-Path $stock 'skse64_loader.exe' }
    $already = @(Get-Process -Name 'SkyrimSE' -ErrorAction SilentlyContinue)
    if ($already.Count -gt 0) { throw 'Skyrim is already running. Close it and run StockRoot again.' }

    Write-Host ''
    Write-Host "Launching $launchExe" -ForegroundColor Cyan
    Write-Host 'Get to the main menu, do NOT load or create a save, then quit to desktop.' -ForegroundColor Yellow
    $launchedAt = Get-Date
    Start-Process -FilePath $launchExe -WorkingDirectory $stock
    $proc = $null
    for ($t = 0; $t -lt 90 -and -not $proc; $t++) {
        Start-Sleep -Seconds 2
        $proc = Get-Process -Name 'SkyrimSE' -ErrorAction SilentlyContinue | Select-Object -First 1
    }
    $processPath = $null
    if ($proc) { try { $processPath = $proc.Path } catch { } }
    $ranFromStock = ($processPath -and $processPath.StartsWith($stock, [StringComparison]::OrdinalIgnoreCase))
    if ($proc) {
        Write-Host "SkyrimSE.exe is running from: $processPath"
        Write-Host 'Waiting for Skyrim to close...'
        while (Get-Process -Name 'SkyrimSE' -ErrorAction SilentlyContinue) { Start-Sleep -Seconds 2 }
    } else {
        Write-Warning 'SkyrimSE.exe never appeared within three minutes.'
    }
    $reachedMenu = Read-Host 'Did Skyrim reach the main menu? (y/n)'
    $errors = Read-Host 'Any error dialogs or odd behaviour? (describe, or press Enter for none)'

    # SKSE log for this run
    $skseLog = Join-Parts $myGames 'SKSE' 'skse64.log'
    $skse = [ordered]@{ logPath = $skseLog; ranThisLaunch = $false; head = @() }
    if (Test-Path -LiteralPath $skseLog) {
        $item = Get-Item -LiteralPath $skseLog
        $skse.ranThisLaunch = ($item.LastWriteTime -ge $launchedAt)
        $skse.head = @(Get-Content -LiteralPath $skseLog -TotalCount 25)
    }

    # Did anything write through a hardlink into the real install?
    $originalsChanged = @()
    foreach ($o in $linkedOriginals) {
        $now = Get-Item -LiteralPath $o.path -ErrorAction SilentlyContinue
        if (-not $now -or $now.Length -ne $o.length -or $now.LastWriteTimeUtc -ne $o.lastWriteUtc) { $originalsChanged += $o.path }
    }
    # Files the game created inside the stock root
    $newInStock = @()
    foreach ($f in @(Get-ChildItem -LiteralPath $stock -Recurse -File -Force -ErrorAction SilentlyContinue)) {
        if ($f.Name -eq $MarkerName) { continue }
        if (-not $created.Contains($f.FullName)) { $newInStock += (Get-RelativePath $stock $f.FullName) }
    }
    # Restore user files, noting which ones the game changed
    $changedUserFiles = @()
    foreach ($b in $backups) {
        $nowHash = $null
        if (Test-Path -LiteralPath $b.original) { $nowHash = (Get-FileHash -LiteralPath $b.original -Algorithm SHA256).Hash }
        if ($nowHash -ne $b.hash) { $changedUserFiles += (Split-Path -Leaf $b.original) }
        Copy-Item -LiteralPath $b.backup -Destination $b.original -Force
    }
    Write-Host "Restored $($backups.Count) user files."

    $report = [ordered]@{
        spikeVersion           = $SpikeVersion
        mode                   = 'StockRoot'
        flavor                 = $install.flavor
        exeVersion             = Get-FileVersionString (Join-Path $source 'SkyrimSE.exe')
        method                 = $method
        steamAppIdFileWritten  = [bool]($AddSteamAppId -and $install.flavor -eq 'Steam')
        stockRoot              = $stock
        build                  = $stats
        launchedVia            = $(if ($useSkse) { 'skse64_loader.exe' } else { 'SkyrimSE.exe' })
        processSeen            = [bool]$proc
        processPath            = $processPath
        ranFromStockRoot       = [bool]$ranFromStock
        reachedMainMenu        = $reachedMenu
        userNotes              = $errors
        skse                   = $skse
        realInstallFilesChanged = $originalsChanged
        filesCreatedInStockRoot = $newInStock
        userFilesChangedByGame = $changedUserFiles
    }
    if ($proc -and -not $ranFromStock) {
        Write-Warning 'Skyrim ran from somewhere other than the stock root (Steam may have relaunched it from its own folder).'
        if ($install.flavor -eq 'Steam' -and -not $AddSteamAppId) { Write-Warning 'Try again with -AddSteamAppId.' }
    }
    Write-Host "The stock root stays at $stock (use -Mode Cleanup to remove it)."
    Save-Report 'stockroot' $report | Out-Null
}

# --------------------------------------------------------------------------------------------
# Mode: ToolRecord
# --------------------------------------------------------------------------------------------

function Get-TreeSnapshot([string]$Dir) {
    $snap = @{}
    if (-not (Test-Path -LiteralPath $Dir)) { return $snap }
    foreach ($f in @(Get-ChildItem -LiteralPath $Dir -Recurse -File -Force -ErrorAction SilentlyContinue)) {
        $id = Get-FileIdentity $f.FullName
        $snap[(Get-RelativePath $Dir $f.FullName)] = [pscustomobject]@{
            length = $f.Length
            write  = $f.LastWriteTimeUtc.Ticks
            id     = $(if ($id) { $id.Id } else { $null })
            links  = $(if ($id) { $id.Links } else { $null })
        }
    }
    return $snap
}

function Get-TopFolder([string]$Rel, [int]$Depth) {
    $parts = $Rel.Split('\')
    if ($parts.Count -le 1) { return '(root)' }
    return ($parts[0..([math]::Min($Depth, $parts.Count - 1) - 1)] -join '\')
}

function Invoke-ToolRecord {
    Initialize-Native | Out-Null
    $roots = @()
    if ($Root) { $roots = $Root }
    else {
        $install = Select-SkyrimInstall 'SkyrimSteam' 'SkyrimGOG'
        $roots += $install.installDir
        $roots += (Join-Path $env:LOCALAPPDATA $install.appDataName)
        $roots += (Join-Parts (Get-DocumentsPath) 'My Games' $install.appDataName)
        $mo2 = Get-Mo2Setups
        foreach ($inst in @($mo2.instances)) {
            if ($inst.gameName -match 'Skyrim' -and $inst.instanceDir) { $roots += (Join-Path $inst.instanceDir 'overwrite') }
        }
    }
    $roots = @($roots | Where-Object { $_ } | Sort-Object -Unique)
    Write-Host 'Watching:' -ForegroundColor Cyan
    $roots | ForEach-Object { Write-Host "  $_" }
    $toolName = Read-Host 'Which tool are you about to run? (e.g. Nemesis, Pandora, BodySlide)'
    Write-Host 'Taking the "before" snapshot...'
    $before = @{}
    foreach ($r in $roots) { $before[$r] = Get-TreeSnapshot $r }
    Write-Host ''
    Write-Host "Now run $toolName the way you normally do (through Vortex or MO2 is fine)." -ForegroundColor Yellow
    [void](Read-Host 'Press Enter once it has finished and you have closed it')
    Write-Host 'Taking the "after" snapshot...'

    $results = @()
    foreach ($r in $roots) {
        $after = Get-TreeSnapshot $r
        $b = $before[$r]
        $added = @(); $removed = @(); $inPlace = @(); $replaced = @(); $throughLink = @()
        foreach ($k in $after.Keys) {
            if (-not $b.ContainsKey($k)) { $added += $k; continue }
            $o = $b[$k]; $n = $after[$k]
            if ($o.length -eq $n.length -and $o.write -eq $n.write -and $o.id -eq $n.id) { continue }
            if ($o.id -and $n.id -and $o.id -ne $n.id) { $replaced += $k }
            else {
                $inPlace += $k
                if ($o.links -gt 1) { $throughLink += $k }
            }
        }
        foreach ($k in $b.Keys) { if (-not $after.ContainsKey($k)) { $removed += $k } }
        $addedBytes = 0L
        foreach ($k in $added) { $addedBytes += $after[$k].length }
        $byFolder = [ordered]@{}
        foreach ($k in ($added + $inPlace + $replaced)) {
            $top = Get-TopFolder $k 3
            if (-not $byFolder.Contains($top)) { $byFolder[$top] = 0 }
            $byFolder[$top]++
        }
        $results += [pscustomobject]@{
            root                   = $r
            filesBefore            = $b.Count
            filesAfter             = $after.Count
            added                  = $added.Count
            addedMB                = [math]::Round($addedBytes / 1MB, 1)
            removed                = $removed.Count
            modifiedInPlace        = $inPlace.Count
            replaced               = $replaced.Count
            modifiedThroughHardlink = $throughLink.Count
            changesByFolder        = $byFolder
            sampleAdded            = @($added | Sort-Object | Select-Object -First 200)
            sampleRemoved          = @($removed | Sort-Object | Select-Object -First 100)
            sampleModifiedInPlace  = @($inPlace | Sort-Object | Select-Object -First 100)
            sampleReplaced         = @($replaced | Sort-Object | Select-Object -First 100)
            sampleThroughHardlink  = @($throughLink | Sort-Object | Select-Object -First 100)
        }
        Write-Host ("{0}: +{1} added, {2} in place, {3} replaced, -{4} removed, {5} written through a hardlink" -f $r, $added.Count, $inPlace.Count, $replaced.Count, $removed.Count, $throughLink.Count)
    }
    Save-Report 'toolrecord' ([ordered]@{ spikeVersion = $SpikeVersion; mode = 'ToolRecord'; tool = $toolName; roots = $results }) | Out-Null
}

# --------------------------------------------------------------------------------------------
# Modes: LinkArm / LinkCheck / VerifyTest  (how Steam writes files)
# --------------------------------------------------------------------------------------------

function Get-SteamAppOrThrow([int]$Id) {
    if (-not $Id) { throw 'Pass -SteamAppId <id>. The inventory report lists app ids.' }
    $app = @(Get-SteamApps | Where-Object { $_.appId -eq [string]$Id -and $_.installed }) | Select-Object -First 1
    if (-not $app) { throw "Steam app $Id is not installed." }
    return $app
}

function Get-LinkStatePath([int]$Id) { return (Join-Parts $SpikeHome 'state' "links-$Id.json") }

function Invoke-LinkArm([int]$Id) {
    if (-not (Initialize-Native)) { throw 'LinkArm needs the native file helpers.' }
    $app = Get-SteamAppOrThrow $Id
    $source = $app.installDir.TrimEnd('\')
    $side = Join-Parts ([IO.Path]::GetPathRoot($source)) 'AgoraSpike' "links-$Id"
    New-MarkedFolder $side ([ordered]@{ createdBy = 'agora-game-spike'; mode = 'LinkArm'; source = $source; createdUtc = (Get-Date).ToUniversalTime().ToString('o') })
    $entries = @()
    $skippedLinked = 0
    foreach ($f in @(Get-ChildItem -LiteralPath $source -Recurse -File -Force -ErrorAction SilentlyContinue)) {
        $id = Get-FileIdentity $f.FullName
        if (-not $id) { continue }
        if ($id.Links -gt 1) { $skippedLinked++; continue }   # already hardlinked by something else (e.g. Vortex)
        $rel = Get-RelativePath $source $f.FullName
        $dest = Join-Path $side $rel
        $destDir = Split-Path -Parent $dest
        if (-not (Test-Path -LiteralPath $destDir)) { New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
        try { [AgoraSpikeNative]::HardLink($dest, $f.FullName) } catch { continue }
        $entries += [pscustomobject]@{ rel = $rel; length = $f.Length; write = $f.LastWriteTimeUtc.Ticks; id = $id.Id }
    }
    $state = [ordered]@{
        appId = $app.appId; name = $app.name; source = $source; side = $side
        buildIdAtArm = $app.buildId; armedUtc = (Get-Date).ToUniversalTime().ToString('o'); entries = $entries
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent (Get-LinkStatePath $Id)) | Out-Null
    [IO.File]::WriteAllText((Get-LinkStatePath $Id), ($state | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
    Write-Host "Armed $($entries.Count) files of $($app.name) (build $($app.buildId)); $skippedLinked already-linked files left alone."
    Write-Host 'After Steam next updates this game, run:  -Mode LinkCheck -SteamAppId' $Id
    return $state
}

function Invoke-LinkCheck([int]$Id, [string]$FocusRel) {
    $statePath = Get-LinkStatePath $Id
    if (-not (Test-Path -LiteralPath $statePath)) { throw "Nothing armed for $Id. Run -Mode LinkArm first." }
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    $app = Get-SteamAppOrThrow $Id
    $counts = [ordered]@{ unchanged = 0; 'modified-in-place' = 0; replaced = 0; deleted = 0; 'link-missing' = 0 }
    $details = @()
    $focus = $null
    foreach ($e in @($state.entries)) {
        $orig = Join-Path $state.source $e.rel
        $link = Join-Path $state.side $e.rel
        $outcome = 'unchanged'
        if (-not (Test-Path -LiteralPath $orig)) { $outcome = 'deleted' }
        elseif (-not (Test-Path -LiteralPath $link)) { $outcome = 'link-missing' }
        else {
            $idNow = Get-FileIdentity $orig
            $item = Get-Item -LiteralPath $orig
            if ($idNow -and $idNow.Id -ne $e.id) { $outcome = 'replaced' }
            elseif ($item.Length -ne $e.length -or $item.LastWriteTimeUtc.Ticks -ne $e.write) { $outcome = 'modified-in-place' }
        }
        $counts[$outcome]++
        if ($outcome -ne 'unchanged') { $details += [pscustomobject]@{ rel = $e.rel; outcome = $outcome } }
        if ($FocusRel -and $e.rel -eq $FocusRel) { $focus = $outcome }
    }
    $report = [ordered]@{
        spikeVersion   = $SpikeVersion
        mode           = 'LinkCheck'
        appId          = $state.appId
        name           = $state.name
        buildIdAtArm   = $state.buildIdAtArm
        buildIdNow     = $app.buildId
        buildChanged   = ($state.buildIdAtArm -ne $app.buildId)
        armedUtc       = $state.armedUtc
        checkedUtc     = (Get-Date).ToUniversalTime().ToString('o')
        counts         = $counts
        focusFile      = $FocusRel
        focusOutcome   = $focus
        changed        = @($details | Select-Object -First 1000)
    }
    Write-Host ''
    Write-Host ("{0}: build {1} -> {2}" -f $state.name, $state.buildIdAtArm, $app.buildId) -ForegroundColor Cyan
    $counts.GetEnumerator() | ForEach-Object { Write-Host ("  {0,-18} {1}" -f $_.Key, $_.Value) }
    if ($FocusRel) { Write-Host "  corrupted file '$FocusRel' -> $focus" -ForegroundColor Yellow }
    return $report
}

function Invoke-VerifyTest([int]$Id) {
    $app = Get-SteamAppOrThrow $Id
    $warning = ''
    if ($app.appId -in '489830', '377160', '1716740', '22380') {
        $warning = "`nWARNING: this looks like a Bethesda game you may have modded. Steam's verify can also pull a pending update. Prefer a small game you don't mod."
    }
    Confirm-Yes (@"
VerifyTest on $($app.name) will:
  1. Hardlink every file into a side folder (costs no disk until Steam replaces files).
  2. Flip one byte at the end of the smallest file, so Steam has something to repair.
  3. Ask you to run Steam > Library > right-click the game > Properties > Installed Files > Verify integrity.
  4. Report whether Steam rewrote the file in place or replaced it with a new file.$warning
"@)
    $state = Invoke-LinkArm $Id
    $victim = @($state.entries | Where-Object { $_.length -ge 16 -and $_.rel -notlike '*.exe' } | Sort-Object length | Select-Object -First 1)
    if ($victim.Count -eq 0) { throw 'No suitable file to corrupt.' }
    $victim = $victim[0]
    $path = Join-Path $state.source $victim.rel
    $fs = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
    try {
        [void]$fs.Seek(-1, [IO.SeekOrigin]::End)
        $b = $fs.ReadByte()
        [void]$fs.Seek(-1, [IO.SeekOrigin]::End)
        $fs.WriteByte([byte]($b -bxor 0xFF))
    } finally { $fs.Close() }
    # The corruption itself is an in-place write; re-baseline that file so only Steam's repair shows up.
    $info = Get-Item -LiteralPath $path
    $victim.length = $info.Length
    $victim.write = $info.LastWriteTimeUtc.Ticks
    [IO.File]::WriteAllText((Get-LinkStatePath $Id), ($state | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
    Write-Host ''
    Write-Host "Corrupted: $($victim.rel)" -ForegroundColor Yellow
    Write-Host 'Now run Verify integrity of game files in Steam for this game.' -ForegroundColor Yellow
    [void](Read-Host 'Press Enter once Steam says verification finished')
    $report = Invoke-LinkCheck $Id $victim.rel
    $report.mode = 'VerifyTest'
    if ($report.focusOutcome -eq 'unchanged') {
        Write-Warning 'The corrupted file still looks unchanged: Steam may not have repaired it yet. Run verify again, then -Mode LinkCheck.'
    }
    Save-Report 'verifytest' $report | Out-Null
}

# --------------------------------------------------------------------------------------------
# Mode: Cleanup
# --------------------------------------------------------------------------------------------

function Invoke-Cleanup {
    $removed = @()
    foreach ($drive in @(Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        if ($drive.Root -notmatch '^[A-Za-z]:\\$') { continue }
        $spike = Join-Path $drive.Root 'AgoraSpike'
        if (-not (Test-Path -LiteralPath $spike)) { continue }
        foreach ($d in @(Get-ChildItem -LiteralPath $spike -Directory -Force -ErrorAction SilentlyContinue)) {
            if (Test-Path -LiteralPath (Join-Path $d.FullName $MarkerName)) {
                # Removing a hardlink removes that name only; the game's own files are untouched.
                Remove-Item -LiteralPath $d.FullName -Recurse -Force
                $removed += $d.FullName
            }
        }
        if (@(Get-ChildItem -LiteralPath $spike -Force -ErrorAction SilentlyContinue).Count -eq 0) { Remove-Item -LiteralPath $spike -Force }
    }
    $state = Join-Path $SpikeHome 'state'
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Recurse -Force }
    $removed | ForEach-Object { Write-Host "Removed $_" }
    Write-Host "Done. Reports and user-file backups are kept in $SpikeHome."
}

# --------------------------------------------------------------------------------------------

# Dot-sourcing loads the functions without running anything (used by the parser tests).
if ($MyInvocation.InvocationName -eq '.') { return }

switch ($Mode) {
    'Inventory' { Invoke-Inventory }
    'StockRoot' { Invoke-StockRoot }
    'ToolRecord' { Invoke-ToolRecord }
    'LinkArm' { Invoke-LinkArm $SteamAppId | Out-Null }
    'LinkCheck' { Save-Report 'linkcheck' (Invoke-LinkCheck $SteamAppId $null) | Out-Null }
    'VerifyTest' { Invoke-VerifyTest $SteamAppId }
    'Cleanup' { Invoke-Cleanup }
}
