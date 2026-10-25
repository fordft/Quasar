param(
    [Parameter(Mandatory = $true)][int]$QaProcessId,
    [ValidateRange(10, 1800)][int]$Seconds = 600,
    [ValidatePattern('^[a-z0-9-]+$')][string]$Label = 'idle'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$owner = Get-Process -Id $QaProcessId
$allowed = @((Join-Path $repo 'target\webext-qa\Quasar Extensions QA.exe'), (Join-Path $repo 'target\debug\zephium-desktop.exe'))
if ($owner.Path -notin $allowed -or $owner.MainWindowTitle -ne 'Quasar Extensions QA') {
    throw 'The selected process is not this checkout''s isolated QA app.'
}
$results = Join-Path $repo ('target\webext-qa-resources\' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $Label)
New-Item -ItemType Directory -Path $results -Force | Out-Null
$previous = @{}
$clock = [Diagnostics.Stopwatch]::StartNew()
$lastSample = 0.0
$longestGap = 0.0
while ($true) {
    $owner.Refresh()
    if ($owner.HasExited) { throw 'QA exited during measurement.' }
    $inventory = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, CreationDate)
    $births = @{}
    foreach ($entry in $inventory) { $births[[int]$entry.ProcessId] = $entry.CreationDate }
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($QaProcessId)
    do {
        $added = $false
        foreach ($entry in $inventory) {
            $parentId = [int]$entry.ParentProcessId
            # Parent IDs survive parent exit and can be reused by unrelated QA
            # descendants. A real child cannot predate its current parent.
            if ($ids.Contains($parentId) -and $null -ne $entry.CreationDate -and
                $null -ne $births[$parentId] -and $entry.CreationDate -ge $births[$parentId] -and
                $ids.Add([int]$entry.ProcessId)) { $added = $true }
        }
    } while ($added)
    $now = $clock.Elapsed.TotalSeconds
    if ($previous.Count) { $longestGap = [Math]::Max($longestGap, $now - $lastSample) }
    $privateBytes = 0L; $workingSet = 0L; $cpuDelta = 0.0; $count = 0
    $current = @{}
    foreach ($processId in $ids) {
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($null -eq $process) { continue }
        $key = "$processId/$($process.StartTime.Ticks)"
        $cpu = $process.TotalProcessorTime.TotalSeconds
        # Windows PowerShell selects the integer overload for Max(0, double),
        # rounding subsecond CPU deltas away. Keep both arguments explicitly double.
        if ($previous.ContainsKey($key)) { $cpuDelta += [Math]::Max([double]0, [double]($cpu - $previous[$key])) }
        $current[$key] = $cpu
        $privateBytes += $process.PrivateMemorySize64
        $workingSet += $process.WorkingSet64
        $count++
    }
    $percent = if ($previous.Count -and $now -gt $lastSample) { 100 * $cpuDelta / ($now - $lastSample) } else { $null }
    [pscustomobject]@{
        elapsedSeconds = [Math]::Round($now, 2)
        processCount = $count
        privateMiB = [Math]::Round($privateBytes / 1MB, 2)
        workingSetMiB = [Math]::Round($workingSet / 1MB, 2)
        cpuOneCorePercent = $percent
    } | Export-Csv -LiteralPath (Join-Path $results 'samples.csv') -NoTypeInformation -Append
    $previous = $current; $lastSample = $now
    $remaining = $Seconds - $clock.Elapsed.TotalSeconds
    if ($remaining -le 0) { break }
    Start-Sleep -Milliseconds ([int]([Math]::Min(30, $remaining) * 1000))
}
# Working sets double-count shared pages. CPU is a sampled lower bound when
# child processes exit between observations. No debugger is attached.
if ($longestGap -gt 45) {
    Write-Warning "Sampling gap of $([Math]::Round($longestGap, 1)) seconds; continuous idle CPU qualification is incomplete."
}
Write-Output $results
