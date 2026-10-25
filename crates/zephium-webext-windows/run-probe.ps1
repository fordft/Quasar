param(
    [ValidateRange(10, 3600)][int]$IdleSeconds = 600,
    [switch]$SkipResources
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location -LiteralPath $repo
$binary = Join-Path $repo 'target\debug\webext-lab-windows.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Build with cargo build -p zephium-webext-windows --features lab --bin webext-lab-windows first.' }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$results = Join-Path $repo "target\webext-windows-probe\$stamp"
New-Item -ItemType Directory -Path $results -Force | Out-Null
$os = Get-CimInstance Win32_OperatingSystem
$metadata = [ordered]@{
    capturedAt = (Get-Date).ToString('o')
    user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    elevated = ([System.Security.Principal.WindowsPrincipal][System.Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
    os = $os.Caption
    osVersion = $os.Version
    architecture = $os.OSArchitecture
    revision = (git -c "safe.directory=$($repo.Replace('\','/'))" rev-parse HEAD)
    idleSeconds = $IdleSeconds
}
if ($metadata.elevated) { throw 'Run this lab in a normal, non-elevated PowerShell.' }
$metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'environment.json') -Encoding utf8

function Invoke-Lab([string]$Name, [object[]]$Steps, [switch]$Measure) {
    $scenario = Join-Path $results "$Name.scenario.json"
    [System.IO.File]::WriteAllText($scenario, (ConvertTo-Json -InputObject $Steps -Depth 15), [System.Text.UTF8Encoding]::new($false))
    $data = Join-Path $results "$Name.data"
    $stdout = Join-Path $results "$Name.jsonl"
    $stderr = Join-Path $results "$Name.stderr.log"
    Write-Host "Starting $Name"
    $process = Start-Process -FilePath $binary -ArgumentList @("`"$scenario`"", "`"$data`"") -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Retain the process handle before it exits; Windows PowerShell 5.1 can
    # otherwise lose ExitCode for short-lived Start-Process children.
    $processHandle = $process.Handle
    $samples = [System.Collections.Generic.List[object]]::new()
    $browserId = $null
    $deadline = (Get-Date).AddSeconds($IdleSeconds + 180)
    while (-not $process.HasExited) {
        if ((Get-Date) -gt $deadline) {
            $process.Kill()
            throw "Lab $Name exceeded its deadline; the lab process was stopped."
        }
        if ($Measure) {
            if (-not $browserId -and (Test-Path -LiteralPath $stdout)) {
                foreach ($line in (Get-Content -LiteralPath $stdout)) {
                    try {
                        $record = $line | ConvertFrom-Json
                        if ($record.kind -eq 'environment') { $browserId = $record.value.browser_pid; break }
                    } catch { }
                }
            }
            if ($browserId) {
                $inventory = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
                $ids = [System.Collections.Generic.HashSet[int]]::new()
                [void]$ids.Add([int]$browserId)
                [void]$ids.Add([int]$process.Id)
                do {
                    $added = $false
                    foreach ($item in $inventory) {
                        if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $added = $true }
                    }
                } while ($added)
                $sampleTime = (Get-Date).ToString('o')
                foreach ($id in $ids) {
                    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
                    if ($p) {
                        $samples.Add([pscustomobject]@{ time=$sampleTime; pid=$id; name=$p.ProcessName; privateBytes=$p.PrivateMemorySize64; workingSetBytes=$p.WorkingSet64; cpuSeconds=$p.CPU })
                    }
                }
            }
        }
        Start-Sleep -Seconds 2
    }
    $process.WaitForExit()
    if ($samples.Count) { $samples | Export-Csv -LiteralPath (Join-Path $results "$Name.resources.csv") -NoTypeInformation }
    Write-Host "$Name exited $($process.ExitCode)"
    if ($process.ExitCode -ne 0) { Get-Content -LiteralPath $stderr; throw "$Name did not complete" }
}

$baseline = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'scenarios\baseline.json') -Raw | ConvertFrom-Json
Invoke-Lab 'renderer-smoke' $baseline
$smoke = @(Get-Content -LiteralPath (Join-Path $results 'renderer-smoke.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
$render = @($smoke | Where-Object { $_.kind -eq 'step' -and $_.value.label -eq 'baseline renderer' })
if (@($smoke | Where-Object { $_.kind -eq 'process_failed' }).Count -gt 0 -or $render.Count -ne 1 -or -not $render[0].value.ok -or $render[0].value.result.result.value.title -ne 'Quasar lab fixture') {
    throw "The extension-free renderer baseline failed. Extension/resource qualification is blocked. See $results"
}
$diagnostic = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'scenarios\diagnostic.json') -Raw | ConvertFrom-Json
Invoke-Lab 'diagnostic' $diagnostic
foreach ($name in @('access-and-actions', 'worker-lifetime')) {
    $steps = Get-Content -LiteralPath (Join-Path $PSScriptRoot "scenarios\$name.json") -Raw | ConvertFrom-Json
    Invoke-Lab $name $steps
}

$suite = Get-Content -LiteralPath (Join-Path $repo 'crates\zephium-webext-macos\suite.json') -Raw | ConvertFrom-Json
$names = @('Dark Reader','Bitwarden','Vimium','Grammarly','SponsorBlock')
$selected = @($suite | Where-Object { $_.name -in $names })
foreach ($extension in $selected) {
    $package = Join-Path $repo "target\webext-suite\$($extension.id).crx"
    if (-not (Test-Path -LiteralPath $package)) { throw "Missing package: $package" }
    $steps = @(
        @{ load=$package; label='load signed package' },
        @{ list=$true },
        @{ navigate='$ORIGIN/first' },
        @{ sleep=4000 },
        @{ eval="return {url:location.href,darkReader:!!document.querySelector('style.darkreader'),grammarly:!!document.querySelector('grammarly-desktop-integration'),vimium:!!document.querySelector('#vimium-ui-root'),body:document.body.innerText.slice(0,300)}"; label='page effects' },
        @{ view='popup' },
        @{ in='popup'; popup=$true },
        @{ sleep=4000 },
        @{ in='popup'; label='popup runtime and tabs'; eval="const out={url:location.href,body:document.body?.innerText?.slice(0,4000),id:chrome.runtime?.id};try{out.tabs=await chrome.tabs.query({active:true,currentWindow:true})}catch(e){out.error=String(e)}return out" },
        @{ in='popup'; screenshot='popup.png' },
        @{ label='background targets'; cdp='Target.getTargets' }
    )
    if ($extension.check) {
        $steps += @{ navigate=$extension.check.url }
        $steps += @{ sleep=5000 }
        $steps += @{ eval=$extension.check.eval; label='existing suite expectation' }
    }
    Invoke-Lab ($extension.name.Replace(' ','-').ToLowerInvariant()) $steps
}

if (-not $SkipResources) {
    $pages = @(
        @{ navigate='$ORIGIN/first' },
        @{ view='tab2' }, @{ in='tab2'; navigate='$ORIGIN/second' },
        @{ view='tab3' }, @{ in='tab3'; navigate='$ORIGIN/third' }
    )
    Invoke-Lab 'baseline-disabled' (@(@{extensions_enabled=$false}) + $pages + @(@{sleep=60000;label='idle baseline'})) -Measure
    Invoke-Lab 'baseline-enabled' ($pages + @(@{sleep=60000;label='idle baseline'})) -Measure
    $loads = @($selected | Where-Object { $_.name -in @('Dark Reader','Bitwarden','Grammarly') } | ForEach-Object { @{load=(Join-Path $repo "target\webext-suite\$($_.id).crx")} })
    Invoke-Lab 'three-extensions-idle' ($loads + $pages + @(@{sleep=($IdleSeconds * 1000);label='uninstrumented idle'})) -Measure
}
Write-Host "Probe finished. Results: $results"
