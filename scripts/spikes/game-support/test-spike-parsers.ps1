# Exercises the spike's parsers against fake Steam / MO2 / Vortex / game folders.
# Runs anywhere PowerShell does (no Windows APIs):  pwsh -File test-spike-parsers.ps1
$ErrorActionPreference = 'Stop'
if (-not $env:LOCALAPPDATA) { $env:LOCALAPPDATA = [IO.Path]::GetTempPath() }
. (Join-Path $PSScriptRoot 'agora-game-spike.ps1')

$failures = 0
function Assert-Equal($Expected, $Actual, [string]$What) {
    if ("$Expected" -ne "$Actual") { Write-Host "FAIL $What`n  expected: $Expected`n  actual:   $Actual" -ForegroundColor Red; $script:failures++ }
    else { Write-Host "ok   $What" }
}
function New-File([string]$Path, [string]$Content = '') {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    [IO.File]::WriteAllText($Path, $Content)
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('agora-spike-test-' + [guid]::NewGuid().ToString('N'))
try {
    # Steam appmanifest: top-level values win over nested InstalledDepots values.
    $acf = Join-Path $root 'appmanifest_489830.acf'
    New-File $acf @'
"AppState"
{
	"appid"		"489830"
	"name"		"The Elder Scrolls V: Skyrim Special Edition"
	"installdir"		"Skyrim Special Edition"
	"buildid"		"15432678"
	"SizeOnDisk"		"15000000000"
	"AutoUpdateBehavior"		"0"
	"InstalledDepots"
	{
		"489833"
		{
			"manifest"		"123"
			"size"		"1"
		}
	}
}
'@
    $v = Read-VdfValues $acf
    Assert-Equal '489830' $v['appid'] 'vdf appid'
    Assert-Equal 'Skyrim Special Edition' $v['installdir'] 'vdf installdir'
    Assert-Equal 'Always keep this game updated' $AutoUpdateLabels[[string]$v['AutoUpdateBehavior']] 'auto-update label'

    $lib = Join-Path $root 'libraryfolders.vdf'
    New-File $lib @'
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}
'@
    $libs = @(Read-VdfLibraryPaths $lib)
    Assert-Equal 2 $libs.Count 'library count'
    Assert-Equal 'D:\SteamLibrary' $libs[1] 'library path unescaped'

    # MO2 instance
    $mo2 = Join-Path $root 'mo2'
    New-File (Join-Path $mo2 'ModOrganizer.ini') @'
[General]
gameName=Skyrim Special Edition
gamePath=@ByteArray(C:\\Games\\Skyrim Special Edition)
selected_profile=@ByteArray(Default)
version=2.5.2

'@
    New-File (Join-Path $mo2 'mods/SKSE/meta.ini') "[General]`nmodid=30379`ninstallationFile=skse64_2_02_06.7z"
    New-File (Join-Path $mo2 'mods/Address Library/meta.ini') "[General]`nmodid=32444"
    New-File (Join-Path $mo2 'mods/Address Library/SKSE/Plugins/versionlib-1-6-1170-0.bin')
    New-File (Join-Path $mo2 'mods/Manual Thing/meta.ini') "[General]`nmodid=0"
    New-Item -ItemType Directory -Force -Path (Join-Path $mo2 'mods/Visuals_separator') | Out-Null
    New-File (Join-Path $mo2 'profiles/Default/modlist.txt') "# managed`n+SKSE`n+Address Library`n-Manual Thing`n+Visuals_separator`n*DLC: Dawnguard"
    New-File (Join-Path $mo2 'profiles/Default/plugins.txt') "# header`n*Unofficial Skyrim Special Edition Patch.esp`nDisabled.esp"
    New-File (Join-Path $mo2 'profiles/Default/settings.ini') "[General]`nLocalSaves=true"
    New-File (Join-Path $mo2 'overwrite/SKSE/log.txt') 'x'
    $inst = Get-Mo2Instance $mo2 'test'
    Assert-Equal 'C:\Games\Skyrim Special Edition' $inst.gamePath 'mo2 gamePath from @ByteArray'
    Assert-Equal 3 $inst.modCount 'mo2 mods excluding separators'
    Assert-Equal 1 $inst.separators 'mo2 separators'
    Assert-Equal 2 $inst.nexusLinkedMods 'mo2 nexus-linked mods'
    Assert-Equal 1 $inst.modsWithArchive 'mo2 mods with archive'
    Assert-Equal 'versionlib-1-6-1170-0.bin' ($inst.addressLibraries -join ',') 'mo2 address library'
    Assert-Equal 1 $inst.overwriteFiles 'mo2 overwrite files'
    Assert-Equal (Join-Path $mo2 'mods') $inst.modsDir 'mo2 mods folder defaults to the instance'
    Assert-Equal (Join-Path $mo2 'overwrite') $inst.overwriteDir 'mo2 overwrite folder defaults to the instance'
    $p = $inst.profiles[0]
    Assert-Equal '2/1/1' "$($p.modsEnabled)/$($p.modsDisabled)/$($p.unmanaged)" 'mo2 modlist counts'
    Assert-Equal '2/1' "$($p.pluginsListed)/$($p.pluginsEnabled)" 'mo2 plugins counts'
    Assert-Equal 'true' $p.localSaves 'mo2 local saves'

    # Vortex deployment manifest
    $gameDir = Join-Path $root 'game'
    New-File (Join-Path $gameDir 'Data/vortex.deployment.json') (@{
            instance = 'abc'; version = 1; deploymentMethod = 'hardlink'; gameId = 'skyrimse'
            stagingPath = 'C:\Vortex\skyrimse\mods'; targetPath = (Join-Path $gameDir 'Data')
            files = @(
                @{ relPath = 'SKSE\Plugins\a.dll'; source = 'ModA' },
                @{ relPath = 'meshes\b.nif'; source = 'ModB' },
                @{ relPath = 'meshes\c.nif'; source = 'ModB' })
        } | ConvertTo-Json -Depth 4)
    $dep = @(Get-VortexDeployments @($gameDir, (Join-Path $gameDir 'Data')))
    Assert-Equal 1 $dep.Count 'vortex manifests found'
    Assert-Equal 'hardlink/3/2' "$($dep[0].deploymentMethod)/$($dep[0].fileCount)/$($dep[0].modCount)" 'vortex method/files/mods'
    Assert-Equal ([IO.Path]::Combine((Join-Path $gameDir 'Data'), 'meshes\b.nif')) $dep[0].targets[1] 'vortex target path'

    # Engine fingerprints
    $unity = Join-Path $root 'valheim'
    New-File (Join-Path $unity 'UnityPlayer.dll'); New-File (Join-Path $unity 'valheim_Data/globalgamemanagers')
    New-File (Join-Path $unity 'BepInEx/core/x.dll'); New-File (Join-Path $unity 'doorstop_config.ini'); New-File (Join-Path $unity 'winhttp.dll')
    $fp = Get-EngineFingerprint $unity
    Assert-Equal 'unity-mono' ($fp.engine -join ',') 'unity mono engine'
    Assert-Equal 'bepinex,proxy-dll:winhttp.dll,unity-doorstop' ($fp.loaders -join ',') 'unity loaders'

    $il2cpp = Join-Path $root 'btd6'
    New-File (Join-Path $il2cpp 'UnityPlayer.dll'); New-File (Join-Path $il2cpp 'GameAssembly.dll'); New-Item -ItemType Directory -Force -Path (Join-Path $il2cpp 'MelonLoader') | Out-Null
    $fp = Get-EngineFingerprint $il2cpp
    Assert-Equal 'unity-il2cpp|melonloader' "$($fp.engine -join ',')|$($fp.loaders -join ',')" 'unity il2cpp + melonloader'

    $ue = Join-Path $root 'satisfactory'
    New-Item -ItemType Directory -Force -Path (Join-Path $ue 'Engine'), (Join-Path $ue 'FactoryGame/Content/Paks/~mods'), (Join-Path $ue 'FactoryGame/Mods/SML') | Out-Null
    $fp = Get-EngineFingerprint $ue
    Assert-Equal 'unreal' ($fp.engine -join ',') 'unreal engine'
    Assert-Equal 'pak-mods(~mods),satisfactory-mod-loader' ($fp.loaders -join ',') 'unreal loaders'
    Assert-Equal 'FactoryGame\Mods' ($fp.modFolders -join ',') 'unreal mod folder'

    $sky = Join-Path $root 'skyrim'
    New-File (Join-Path $sky 'Data/Skyrim.esm'); New-File (Join-Path $sky 'skse64_loader.exe')
    $fp = Get-EngineFingerprint $sky
    Assert-Equal 'creation|skse64' "$($fp.engine -join ',')|$($fp.loaders -join ',')" 'creation engine + skse'

    $ck3 = Join-Path $root 'ck3'
    New-File (Join-Path $ck3 'launcher/launcher-settings.json')
    Assert-Equal 'paradox' ((Get-EngineFingerprint $ck3).engine -join ',') 'paradox'

    $godot = Join-Path $root 'brotato'
    New-File (Join-Path $godot 'Brotato.pck')
    Assert-Equal 'godot' ((Get-EngineFingerprint $godot).engine -join ',') 'godot'

    # Redaction
    Assert-Equal '"C:\\Users\\<user>\\Documents"' (Protect-Text '"C:\\Users\\alice\\Documents"') 'redact json path'
    Assert-Equal 'C:\Users\<user>\OneDrive' (Protect-Text 'C:\Users\alice\OneDrive') 'redact plain path'

    # MO2 instance scan: depth limit, system folders skipped, found folders not descended into
    $scan = Join-Path $root 'drive'
    New-File (Join-Path $scan 'Skyrim-MO2-Salvage/MO2/ModOrganizer.ini')
    New-File (Join-Path $scan 'Games/Wabbajack/List/ModOrganizer.ini')
    New-File (Join-Path $scan 'Too/Deep/For/Scan/ModOrganizer.ini')
    New-File (Join-Path $scan 'Program Files/MO2/ModOrganizer.ini')
    $hits = @(Find-Mo2Instances 3 @($scan) | ForEach-Object { Get-RelativePath $scan $_ } | Sort-Object)
    Assert-Equal 'Games/Wabbajack/List|Skyrim-MO2-Salvage/MO2' (($hits -join '|') -replace '\\', '/') 'mo2 scan finds instances up to depth 3 only'

    # Log head: plain strings, bounded count
    $log = Join-Path $root 'skse64.log'
    New-File $log "line one`r`nline two`r`nline three"
    $head = @(Read-HeadLines $log 2)
    Assert-Equal 'line one|line two' ($head -join '|') 'log head reads the first lines'
    Assert-Equal 'System.String' $head[0].GetType().FullName 'log head lines are plain strings'
    Assert-Equal 0 @($head[0].PSObject.Properties | Where-Object { $_.Name -eq 'PSPath' }).Count 'log head lines carry no provider properties'

    # MicrosoftGame.config
    $cfg = Join-Path $root 'MicrosoftGame.config'
    New-File $cfg @'
<?xml version="1.0" encoding="utf-8"?>
<Game configVersion="1">
  <Identity Name="ParadoxInteractive.ProjectTitus" Publisher="CN=Paradox" Version="1.1.289.0" />
  <ExecutableList>
    <Executable Name="binaries\ck3.exe" Id="Game" TargetDeviceFamily="PC" />
  </ExecutableList>
  <DesktopRegistration>
    <ModFolder>mods</ModFolder>
    <EnableWritesToPackageRoot>true</EnableWritesToPackageRoot>
  </DesktopRegistration>
  <ShellVisuals DefaultDisplayName="Crusader Kings III" />
</Game>
'@
    $gc = Read-GameConfig $cfg
    Assert-Equal 'ParadoxInteractive.ProjectTitus' $gc.identityName 'game config identity'
    Assert-Equal 'binaries\ck3.exe/Game' "$($gc.executables[0].name)/$($gc.executables[0].id)" 'game config executable'
    Assert-Equal 'mods|true' "$($gc.desktopRegistration['ModFolder'])|$($gc.desktopRegistration['EnableWritesToPackageRoot'])" 'game config desktop registration'
    Assert-Equal 'Identity,ExecutableList,DesktopRegistration,ShellVisuals' ($gc.topLevelElements -join ',') 'game config elements'
    $dlc = Join-Path $root 'dlc.config'
    New-File $dlc '<Game configVersion="1"><Identity Name="X.DLC" Publisher="p" Version="1.0.0.0" /></Game>'
    Assert-Equal 0 @((Read-GameConfig $dlc).executables).Count 'dlc config has no executables'

    # Executable readability
    $exe = Join-Path $root 'game.exe'
    [IO.File]::WriteAllBytes($exe, [byte[]](0x4D, 0x5A, 0x90, 0x00))
    $noise = Join-Path $root 'encrypted.exe'
    [IO.File]::WriteAllBytes($noise, [byte[]](0x13, 0x37, 0x00))
    Assert-Equal 'True|True' "$((Test-ReadableExecutable $exe).readable)|$((Test-ReadableExecutable $exe).peHeader)" 'plain exe has a PE header'
    Assert-Equal 'True|False' "$((Test-ReadableExecutable $noise).readable)|$((Test-ReadableExecutable $noise).peHeader)" 'encrypted exe reads as noise'
    Assert-Equal 'False' (Test-ReadableExecutable (Join-Path $root 'missing.exe')).exists 'missing exe'

    Assert-Equal 'FactoryGame\Mods\SML' (Get-TopFolder 'FactoryGame\Mods\SML\x.dll' 3) 'top folder depth 3'
    Assert-Equal '(root)' (Get-TopFolder 'x.dll' 3) 'top folder root file'
}
finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
# PowerShell names are case-insensitive, so `$id = ...` inside a function taking `[int]$Id`
# assigns to the typed parameter and fails at run time (it broke LinkArm on Windows).
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'agora-game-spike.ps1'), [ref]$null, [ref]$null)
foreach ($fn in $ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
    $params = @()
    if ($fn.Parameters) { $params += $fn.Parameters }
    if ($fn.Body.ParamBlock) { $params += $fn.Body.ParamBlock.Parameters }
    foreach ($p in $params) {
        if (-not ($p.Attributes | Where-Object { $_ -is [System.Management.Automation.Language.TypeConstraintAst] })) { continue }
        $name = $p.Name.VariablePath.UserPath
        $writes = @($fn.Body.FindAll({ param($n)
                    $n -is [System.Management.Automation.Language.AssignmentStatementAst] -and
                    $n.Left -is [System.Management.Automation.Language.VariableExpressionAst] -and
                    $n.Left.VariablePath.UserPath -eq $name }, $true))
        Assert-Equal 0 $writes.Count "$($fn.Name): typed parameter `$$name is never reassigned"
    }
}

if ($failures -gt 0) { Write-Host "$failures failure(s)" -ForegroundColor Red; exit 1 }
Write-Host 'All parser checks passed.' -ForegroundColor Green
