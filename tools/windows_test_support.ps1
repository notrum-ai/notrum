# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
# Process/clock adapters let the runner's failure paths be tested without a desktop.

function Get-NativeTime {
    [Diagnostics.Stopwatch]::GetTimestamp() / [double][Diagnostics.Stopwatch]::Frequency
}

function Throw-NativeFailure([string]$Reason) {
    $failure = [InvalidOperationException]::new('Native test operation failed.')
    $failure.Data['NotrumReason'] = $Reason
    throw $failure
}

function Get-NativeFailureReason($Failure) {
    $allowed = @('test/timeout', 'test/failed', 'window/timeout', 'state/timeout',
        'process/early/exit', 'process/exit/code', 'process/close', 'process/cleanup',
        'state/mismatch', 'content/changed')
    $reason = $Failure.Exception.Data['NotrumReason']
    if ($reason -in $allowed) { return $reason }
    return 'runner/error'
}

function Wait-NativeReady {
    param($Process, [scriptblock]$State, $Record,
        [scriptblock]$Now = { Get-NativeTime },
        [scriptblock]$Pause = { param($Milliseconds) Start-Sleep -Milliseconds $Milliseconds })
    $deadline = (& $Now) + 60
    while ($true) {
        $Process.Refresh()
        if ($Process.HasExited) { Throw-NativeFailure 'process/early/exit' }
        $Record.stage = 'window'
        $ready = $Process.MainWindowHandle -ne [IntPtr]::Zero -and $Process.Responding
        if ($ready) { $Record.stage = 'state' }
        if ((& $Now) -ge $deadline) { Throw-NativeFailure ($Record.stage + '/timeout') }
        if ($ready -and (& $State)) {
            if ((& $Now) -ge $deadline) { Throw-NativeFailure 'state/timeout' }
            $Process.Refresh()
            if ($Process.HasExited) { Throw-NativeFailure 'process/early/exit' }
            return
        }
        & $Pause 100
    }
}

function Close-NativeProcess($Process) {
    $Process.Refresh()
    if ($Process.HasExited) { Throw-NativeFailure 'process/early/exit' }
    if (-not $Process.CloseMainWindow() -or -not $Process.WaitForExit(30000)) {
        Throw-NativeFailure 'process/close'
    }
    if ($Process.ExitCode -ne 0) { Throw-NativeFailure 'process/exit/code' }
}

function Stop-OwnedNativeProcess($Process) {
    if ($null -eq $Process) { return }
    try {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            try { $Process.Kill() } catch [InvalidOperationException] {
                $Process.Refresh()
                if (-not $Process.HasExited) { throw }
            }
            if (-not $Process.WaitForExit(10000)) { Throw-NativeFailure 'process/cleanup' }
        }
    } finally { $Process.Dispose() }
}

function ConvertTo-NativeComparisonPath([string]$Path) {
    if ($Path.StartsWith('\\?\')) { return $Path.Substring(4) }
    return $Path
}

function Test-NativeSettings {
    param([string]$Path, [string[]]$ExternalPaths = @(), [string]$SelectedNote = '',
        [scriptblock]$Read = { param($Name) [IO.File]::ReadAllText($Name) })
    try { $text = & $Read $Path } catch [IO.IOException] { return $false }
    try { $settings = $text | ConvertFrom-Json -ErrorAction Stop } catch [ArgumentException] { return $false }
    if ($null -eq $settings -or $null -eq $settings.PSObject.Properties['version'] -or
        $settings.version -ne 1 -or $null -eq $settings.PSObject.Properties['window'] -or
        $null -eq $settings.PSObject.Properties['sidebar'] -or
        $null -eq $settings.PSObject.Properties['external_files'] -or
        $null -eq $settings.PSObject.Properties['selected_external']) { return $false }
    if ($SelectedNote -ne '' -and ($null -eq $settings.PSObject.Properties['selected_note'] -or
        $settings.selected_note -cne $SelectedNote)) { return $false }
    $files = @($settings.external_files)
    if ($files.Count -ne $ExternalPaths.Count) { return $false }
    for ($index = 0; $index -lt $files.Count; $index++) {
        $file = $files[$index]
        if ($null -eq $file -or $null -eq $file.PSObject.Properties['engine_id'] -or
            $null -eq $file.PSObject.Properties['absolute_path'] -or $file.engine_id -cne 'markdown' -or
            (ConvertTo-NativeComparisonPath $file.absolute_path) -ine $ExternalPaths[$index]) { return $false }
    }
    if ($ExternalPaths.Count -eq 0) { return $null -eq $settings.selected_external }
    return (ConvertTo-NativeComparisonPath $settings.selected_external) -ieq $ExternalPaths[0]
}

function Invoke-NativeSmoke {
    param([string]$Application, [string[]]$Arguments, [scriptblock]$State, $Record,
        [scriptblock]$Start = { param($Executable, $Arguments)
            Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru
        })
    $process = $null
    $started = Get-NativeTime
    $Record.stage = 'start'
    $Record.reason = 'none'
    try {
        $process = & $Start $Application $Arguments
        # Cache the owned handle so ExitCode remains available after termination.
        $null = $process.Handle
        Wait-NativeReady -Process $process -State $State -Record $Record
        $Record.stage = 'close'
        Close-NativeProcess $process
        $Record.exitCode = $process.ExitCode
        $Record.stage = 'verify'
        if (-not (& $State)) { Throw-NativeFailure 'state/mismatch' }
        $Record.stage = 'complete'
    } catch {
        $Record.reason = Get-NativeFailureReason $_
        throw
    } finally {
        try { Stop-OwnedNativeProcess $process } catch {
            $Record.cleanup = 'failed'
            if ($Record.reason -eq 'none') {
                $Record.reason = 'process/cleanup'
                throw
            }
        } finally {
            $Record.durationMs = [long](([Math]::Max(0, (Get-NativeTime) - $started)) * 1000)
        }
    }
}

function Invoke-NativeTest {
    param([string]$Executable, [string]$Log,
        [scriptblock]$Start = { param($Name, $OutputPath)
            Start-Process -FilePath $Name -ArgumentList '--test-threads=1' -PassThru -NoNewWindow -RedirectStandardOutput $OutputPath -RedirectStandardError ($OutputPath + '.stderr')
        })
    $process = $null
    $started = Get-NativeTime
    $record = [ordered]@{ executable = [IO.Path]::GetFileName($Executable)
        exitCode = $null; stage = 'rust'; reason = 'none'; durationMs = 0 }
    try {
        $process = & $Start $Executable $Log
        $null = $process.Handle
        if (-not $process.WaitForExit(600000)) {
            $record.reason = 'test/timeout'
        } else {
            $record.exitCode = $process.ExitCode
            if ($process.ExitCode -ne 0) { $record.reason = 'test/failed' }
        }
    } catch {
        $record.reason = Get-NativeFailureReason $_
    } finally {
        try { Stop-OwnedNativeProcess $process } catch {
            $record.cleanup = 'failed'
            if ($record.reason -eq 'none') { $record.reason = 'process/cleanup' }
        }
        $record.durationMs = [long](([Math]::Max(0, (Get-NativeTime) - $started)) * 1000)
    }
    return $record
}

function Invoke-NativeTestSuite {
    param([string[]]$Executables, [string]$Directory, [string]$LogDirectory, $Report,
        [scriptblock]$AfterEach, [scriptblock]$Run = {
            param($Executable, $Log) Invoke-NativeTest -Executable $Executable -Log $Log
        })
    foreach ($name in $Executables) {
        if ([IO.Path]::GetFileName($name) -ne $name -or $name -notmatch '^[A-Za-z0-9_]+-[a-f0-9]+\.exe$') {
            throw 'Invalid test executable name.'
        }
        $log = Join-Path $LogDirectory ($name + '.log')
        $entry = & $Run (Join-Path $Directory $name) $log
        # Retain the result even if processing its diagnostics subsequently fails.
        $Report.tests += $entry
        & $AfterEach $entry $log
    }
}
