# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
# Run with: powershell -NoProfile -ExecutionPolicy Bypass -File .\Run-Tests.ps1
[CmdletBinding()]
param([switch]$Interactive)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not [Environment]::Is64BitOperatingSystem -or [Environment]::OSVersion.Version.Build -lt 10240) {
    throw 'Windows 10/11 x64 is required.'
}
$drive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot([IO.Path]::GetTempPath()))
if ($drive.DriveType -ne [IO.DriveType]::Fixed -or $drive.DriveFormat -ne 'NTFS') {
    throw 'The test temporary directory must be on local NTFS.'
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('Notrum Windows 日本語 ' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$previous = @{}
foreach ($name in @('TEMP', 'TMP', 'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH', 'NOTRUM_TEST_JUNCTION')) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$report = [ordered]@{
    platform = 'Windows'; build = [Environment]::OSVersion.Version.ToString()
    architecture = $env:PROCESSOR_ARCHITECTURE; filesystem = $drive.DriveFormat
    started = [DateTime]::UtcNow.ToString('o'); tests = @(); status = 'running'
    interactive = 'not performed'; temporaryWorkspace = $root
}
try {
    $env:TEMP = $root
    $env:TMP = $root
    $env:USERPROFILE = $root
    $env:HOMEDRIVE = [IO.Path]::GetPathRoot($root).TrimEnd('\')
    $env:HOMEPATH = $root.Substring($env:HOMEDRIVE.Length)
    $junctionTarget = Join-Path $root 'junction target'
    New-Item -ItemType Directory -Path $junctionTarget | Out-Null
    $env:NOTRUM_TEST_JUNCTION = Join-Path $root 'junction'
    New-Item -ItemType Junction -Path $env:NOTRUM_TEST_JUNCTION -Target $junctionTarget | Out-Null
    $executables = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'tests.json') | ConvertFrom-Json
    foreach ($name in $executables) {
        if ([IO.Path]::GetFileName($name) -ne $name -or -not $name.EndsWith('.exe')) {
            throw "Invalid test executable name: $name"
        }
        Write-Host "Running $name"
        $log = Join-Path $root ($name + '.log')
        & (Join-Path $PSScriptRoot $name) --test-threads=1 2>&1 | Tee-Object -FilePath $log
        $code = $LASTEXITCODE
        $report.tests += [ordered]@{ executable = $name; exitCode = $code; log = $log }
        if ($code -ne 0) { throw "Test executable failed: $name (exit $code)" }
    }
    $application = Join-Path (Split-Path -Parent $PSScriptRoot) 'Notrum.exe'
    $report.applicationSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $application).Hash
    $report.applicationVersion = (Get-Item -LiteralPath $application).VersionInfo.FileVersion
    $workspace = Join-Path $root 'Workspace with spaces 日本語'
    New-Item -ItemType Directory -Path (Join-Path $workspace 'notes') -Force | Out-Null
    if (-not (Test-Path -LiteralPath $application)) { throw 'Notrum.exe is missing beside the test package.' }
    $process = Start-Process -FilePath $application -ArgumentList @(('"' + $workspace + '"'), '--smoke-exit-ms', '1800') -PassThru
    if (-not $process.WaitForExit(30000)) {
        $process.Kill()
        throw 'The native application smoke test timed out.'
    }
    if ($process.ExitCode -ne 0) { throw "The native application exited with $($process.ExitCode)." }
    $external = Join-Path $root 'External 日本語 #1.MD'
    $second = Join-Path $root 'External two.txt'
    [IO.File]::WriteAllText($external, "External unchanged`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($second, "Second unchanged`n", [Text.UTF8Encoding]::new($false))
    $arguments = @('--workspace', ('"' + $workspace + '"'), '--open', ('"' + $external + '"'), ('"' + $second + '"'), '--smoke-exit-ms', '1800')
    $process = Start-Process -FilePath $application -ArgumentList $arguments -PassThru
    if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'External file launch timed out.' }
    if ($process.ExitCode -ne 0) { throw 'External file launch failed.' }
    $settings = Get-Content -Raw -LiteralPath (Join-Path $workspace '.notrum/settings.json') | ConvertFrom-Json
    $selectedPath = $settings.selected_external
    if ($selectedPath.StartsWith('\\?\')) { $selectedPath = $selectedPath.Substring(4) }
    if ($settings.external_files.Count -ne 2 -or $selectedPath -ne $external) { throw 'External file order/selection differs.' }
    if ([IO.File]::ReadAllText($external) -ne "External unchanged`n") { throw 'Opening modified external content.' }
    $report.externalLaunch = 'passed'
    $registration = Join-Path (Split-Path -Parent $PSScriptRoot) 'Register.ps1'
    $testRegistry = 'Software\NotrumTests\' + [Guid]::NewGuid().ToString('N')
    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($testRegistry + '\.txt')
        $key.SetValue('', 'Other.TextEditor')
        $key.Dispose()
        & $registration -RegistryRoot $testRegistry
        & $registration -RegistryRoot $testRegistry
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($testRegistry + '\Notrum.Document\shell\open\command')
        if ($key.GetValue('') -ne ('"' + $application + '" --open "%1"')) { throw 'Invalid Open With command.' }
        $key.Dispose()
        & $registration -RegistryRoot $testRegistry -Remove
        & $registration -RegistryRoot $testRegistry -Remove
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($testRegistry + '\.txt')
        if ($key.GetValue('') -ne 'Other.TextEditor') { throw 'Registration changed a default association.' }
        $key.Dispose()
        $report.registration = 'passed in isolated registry subtree'
    } finally {
        [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($testRegistry, $false)
    }
    $report.nativeSmoke = 'passed'
    if ($Interactive) {
        Write-Host 'Complete the Windows UI checklist in docs/windows.md. This script does not mark manual checks as passed.'
        Start-Process -FilePath $application -ArgumentList ('"' + $workspace + '"') -Wait
        $report.interactive = 'opened; checklist results must be recorded separately'
    }
    $report.status = 'automated tests passed'
} catch {
    $report.status = 'failed'
    $report.error = $_.Exception.Message
    throw
} finally {
    $report.finished = [DateTime]::UtcNow.ToString('o')
    $reportPath = Join-Path $PSScriptRoot 'windows-results.json'
    $report | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 -LiteralPath $reportPath
    foreach ($name in $previous.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
    }
    Write-Host "Results: $reportPath"
    Write-Host "Test workspace and logs retained: $root"
}
