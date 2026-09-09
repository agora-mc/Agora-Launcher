# Offline regression checks: exercise the real hook with a fake signing tool.
# No Azure calls or real signatures are produced.
$ErrorActionPreference = 'Stop'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('agora-signing-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testDir | Out-Null
$savedEnvironment = @{}
foreach ($name in @('AGORA_SIGNTOOL_PATH', 'AGORA_SIGNING_DLIB', 'AGORA_SIGNING_METADATA')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    $fakeTool = Join-Path $testDir 'fake signtool.ps1'
    @'
$global:signingCalls.Add(@($args))
$global:LASTEXITCODE = if ($args[0] -eq $global:failOperation) { 9 } else { 0 }
'@ | Set-Content -LiteralPath $fakeTool
    $file = Join-Path $testDir 'Agora [test] & space.exe'
    Set-Content -LiteralPath $file -Value 'unsigned fixture'
    $env:AGORA_SIGNTOOL_PATH = $fakeTool
    $env:AGORA_SIGNING_DLIB = $file
    $env:AGORA_SIGNING_METADATA = $file
    function Get-AuthenticodeSignature {
        param([string]$LiteralPath)
        if ($LiteralPath -ne $file) { throw 'Verification received an altered file path.' }
        [pscustomobject]@{
            Status = $global:signatureStatus
            TimeStamperCertificate = $global:timestampCertificate
        }
    }
    $hook = Join-Path $PSScriptRoot 'sign-windows.ps1'
    $cases = @(
        @{ Name = 'valid timestamped signature'; Fail = ''; Status = 'Valid'; Timestamp = 'present'; Error = $false; Calls = 2 }
        @{ Name = 'signer failure'; Fail = 'sign'; Status = 'Valid'; Timestamp = 'present'; Error = $true; Calls = 1 }
        @{ Name = 'verification failure'; Fail = 'verify'; Status = 'Valid'; Timestamp = 'present'; Error = $true; Calls = 2 }
        @{ Name = 'unsigned output'; Fail = ''; Status = 'NotSigned'; Timestamp = 'present'; Error = $true; Calls = 2 }
        @{ Name = 'missing timestamp'; Fail = ''; Status = 'Valid'; Timestamp = $null; Error = $true; Calls = 2 }
    )
    foreach ($case in $cases) {
        $global:signingCalls = [Collections.Generic.List[object]]::new()
        $global:failOperation = $case.Fail
        $global:signatureStatus = $case.Status
        $global:timestampCertificate = $case.Timestamp
        $failed = $false
        try { & $hook -FilePath $file } catch { $failed = $true }
        if ($failed -ne $case.Error -or $global:signingCalls.Count -ne $case.Calls) {
            throw "Failed case: $($case.Name)"
        }
        foreach ($call in $global:signingCalls) {
            if ($call[-1] -cne $file) { throw 'File path was not passed as one literal argument.' }
        }
        Write-Host "PASS: $($case.Name)"
    }
    $env:AGORA_SIGNING_METADATA = ''
    $global:signingCalls.Clear()
    $failed = $false
    try { & $hook -FilePath $file } catch { $failed = $true }
    if (-not $failed -or $global:signingCalls.Count -ne 0) { throw 'Missing setup did not fail before signing.' }
    Write-Host 'PASS: missing setup fails before signing'
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name])
    }
    # Delete only this test's explicitly tracked files and its now-empty directory.
    foreach ($path in @((Join-Path $testDir 'fake signtool.ps1'), (Join-Path $testDir 'Agora [test] & space.exe'))) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    Remove-Item -LiteralPath $testDir
}
