$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$results = Join-Path $repo ('target\webext-worker-compat\' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$fixture = Join-Path $results 'fixture'
$hostDir = Join-Path $fixture 'zephium-windows-host'
New-Item -ItemType Directory -Path $hostDir -Force | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
# Reuse the public lab key, never an installed extension's storage or identity.
$manifest = Get-Content (Join-Path $PSScriptRoot 'fixtures\storage\manifest.json') -Raw | ConvertFrom-Json
$manifest.name = 'Quasar browser API qualifier'
$manifest.PSObject.Properties.Remove('content_scripts')
$manifest.action = @{default_title='Action qualifier'}
[IO.File]::WriteAllText((Join-Path $fixture 'manifest.json'), ($manifest | ConvertTo-Json -Depth 10), $utf8)
Copy-Item (Join-Path $PSScriptRoot 'fixtures\browser-api\worker.js') (Join-Path $fixture 'worker.js')
Copy-Item (Join-Path $repo 'crates\zephium-webext\src\windows\worker-compat.js') (Join-Path $fixture 'compat.js')
[IO.File]::WriteAllText((Join-Path $fixture 'popup.html'), '<!doctype html><title>Options qualifier</title>', $utf8)
[IO.File]::WriteAllText((Join-Path $hostDir 'host.html'), '<!doctype html><title>API qualifier</title>', $utf8)
$steps = @(
    @{human_visible=$true;adopt_new_windows=$true;navigate='$ORIGIN/form'},
    @{load=$fixture}, @{view='host';window=$true},
    @{in='host';navigate='chrome-extension://$ID/zephium-windows-host/host.html'}, @{sleep=1000},
    @{in='host';eval='globalThis.results=[]; for (const callback of [false,true]) chrome.runtime.sendMessage({op:"create",url:"$ORIGIN/same",callback}).then(result=>results.push(result)); return "started";'},
    @{sleep=2000},
    @{in='host';label='creates';eval='return {results,worker:await chrome.runtime.sendMessage({op:"results"})};'},
    @{in='host';capture_human=$true},
    @{in='host';label='click';eval='const [tab]=await chrome.tabs.query({windowId:$HUMAN_WINDOW}); return await chrome.runtime.sendMessage({__zephiumActionClick:true,tabId:tab.id});'},
    @{sleep=500}, @{in='host';label='clicked';eval='return await chrome.runtime.sendMessage({op:"results"});'}
)
$scenario = Join-Path $results 'scenario.json'
[IO.File]::WriteAllText($scenario, (ConvertTo-Json -InputObject $steps -Depth 12), $utf8)
$stdout = Join-Path $results 'results.jsonl'
$process = Start-Process -FilePath (Join-Path $repo 'target\debug\webext-lab-windows.exe') -ArgumentList @("`"$scenario`"", "`"$(Join-Path $results 'data')`"") -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError (Join-Path $results 'stderr.log')
$processHandle = $process.Handle
if (-not $process.WaitForExit(45000)) { $process.Kill(); throw 'Worker qualifier timed out.' }
if ($process.ExitCode -ne 0) { throw 'Worker qualifier failed.' }
$records = @(Get-Content $stdout | ForEach-Object { $_ | ConvertFrom-Json })
$creates = ($records | Where-Object { $_.value.label -eq 'creates' }).value.result.result.value
if ($creates.results.Count -ne 2 -or @($creates.results | Where-Object { $null -eq $_.tab.id }).Count -ne 0 -or $creates.results[0].tab.id -eq $creates.results[1].tab.id) { throw 'Native tab IDs were not returned.' }
$clicked = ($records | Where-Object { $_.value.label -eq 'clicked' }).value.result.result.value
if ($clicked.clicks.Count -ne 1) { throw 'Action listener was not called exactly once.' }
Write-Output "Worker create and action qualifiers passed: $results"
