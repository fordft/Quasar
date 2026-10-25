# Run in a fresh PowerShell process; mocks replace OS process queries only.
$ErrorActionPreference = 'Stop'
$fixtureBirth = [datetime]'2026-01-01T12:00:00'
$fixtureCalls = @{}
$fixtureRepo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

function Get-CimInstance {
    param($ClassName)
    if ($ClassName -ne 'Win32_Process') { throw 'Unexpected process query.' }
    # Reverse order exercises recursive discovery. PID 2 was reused: the old
    # browser (4) and its child (5) must not join the new QA process tree.
    @(
        @{ ProcessId = 5; ParentProcessId = 4; Offset = -9 },
        @{ ProcessId = 4; ParentProcessId = 2; Offset = -10 },
        @{ ProcessId = 3; ParentProcessId = 2; Offset = 2 },
        @{ ProcessId = 2; ParentProcessId = 1; Offset = 1 },
        @{ ProcessId = 1; ParentProcessId = 99; Offset = 0 }
    ) | ForEach-Object {
        [pscustomobject]@{
            ProcessId = $_.ProcessId
            ParentProcessId = $_.ParentProcessId
            CreationDate = $fixtureBirth.AddSeconds($_.Offset)
        }
    }
}

function Get-Process {
    param([int]$Id, $ErrorAction)
    if ($Id -lt 1 -or $Id -gt 5) { throw 'Unexpected process ID.' }
    $fixtureCalls[$Id] = 1 + $fixtureCalls[$Id]
    $process = [pscustomobject]@{
        Path = Join-Path $fixtureRepo 'target\webext-qa\Quasar Extensions QA.exe'
        MainWindowTitle = 'Quasar Extensions QA'
        HasExited = $false
        StartTime = $fixtureBirth.AddSeconds($Id - 1)
        TotalProcessorTime = [timespan]::FromSeconds(0.2 * $fixtureCalls[$Id])
        PrivateMemorySize64 = 1MB
        WorkingSet64 = 2MB
    }
    $process | Add-Member -MemberType ScriptMethod -Name Refresh -Value { }
    $process
}

$results = & (Join-Path $PSScriptRoot 'measure-webext-qa.ps1') -QaProcessId 1 -Seconds 10 -Label sampler-regression
$samples = @(Import-Csv (Join-Path $results 'samples.csv'))
if ($samples.Count -lt 2) { throw 'Expected initial and final samples.' }
foreach ($sample in $samples) {
    if ([int]$sample.processCount -ne 3 -or [double]$sample.privateMiB -ne 3 -or
        [double]$sample.workingSetMiB -ne 6) {
        throw 'Reused parent PID contaminated the sampled process tree.'
    }
}
$elapsed = [double]$samples[1].elapsedSeconds - [double]$samples[0].elapsedSeconds
$cpuSeconds = [double]$samples[1].cpuOneCorePercent * $elapsed / 100
if ([Math]::Abs($cpuSeconds - 0.6) -gt 0.01) {
    throw "Expected 0.6 fractional CPU seconds across three processes; got $cpuSeconds."
}
Write-Output 'PASS: recursive ownership excludes reused-parent descendants and retains fractional CPU time.'
