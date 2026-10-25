$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$binary = Join-Path $repo 'target\debug\webext-lab-windows.exe'
$results = Join-Path $repo ('target\webext-browser-surfaces\' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $results -Force | Out-Null
function Package([string]$Id) {
    $path = Join-Path $repo "target\webext-suite\$Id.crx"
    if (-not (Test-Path -LiteralPath $path)) { throw "Missing signed package: $path" }
    $path
}
function Run-Case([string]$Name, [object[]]$Steps) {
    $scenario = Join-Path $results "$Name.json"
    [IO.File]::WriteAllText($scenario, (ConvertTo-Json -InputObject $Steps -Depth 10), [Text.UTF8Encoding]::new($false))
    $stdout = Join-Path $results "$Name.jsonl"
    $process = Start-Process -FilePath $binary -ArgumentList @("`"$scenario`"", "`"$(Join-Path $results "$Name.data")`"") -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError (Join-Path $results "$Name.stderr.log")
    $processHandle = $process.Handle
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw "Lab timed out: $Name" }
    if ($process.ExitCode -ne 0) { throw "Lab failed: $Name" }
    @(Get-Content $stdout | ForEach-Object { $_ | ConvertFrom-Json })
}
$grammar = Run-Case 'grammarly' @(
    @{human_visible=$true;adopt_new_windows=$true;navigate='$ORIGIN/form'},
    @{load=(Package 'kbfnbcaeplbcioakkpcpgfkobkghlhen');prepare_windows=$true},
    @{view='manager'}, @{in='manager';navigate='chrome-extension://$ID/zephium-windows-host/host.html'},
    @{sleep=2000}, @{in='manager';capture_human=$true},
    @{view='popup';window=$true;target_human=$true}, @{in='popup';popup=$true}, @{sleep=5000},
    # Diagnostic DOM click, not evidence of physical user activation. No login is submitted.
    @{in='popup';eval="document.querySelector('a[href*=signin]').click(); return 'clicked';"},
    @{sleep=8000}
)
if (-not ($grammar | Where-Object { $_.kind -eq 'native_adoption' -and $_.value.url -like 'https://auth.grammarly.com/*' })) { throw 'No native OAuth adoption.' }
if (-not ($grammar | Where-Object { $_.kind -eq 'document_loaded' -and $_.value.url -like 'https://www.grammarly.com/signin*' })) { throw 'Sign-in document did not load.' }
$menu = Run-Case 'context-menu' @(
    @{human_visible=$true;navigate='$ORIGIN/form'},
    @{load=(Package 'nngceckbapebfimnlniiiahkandclblb')},
    @{view='observer'}, @{in='observer';popup=$true}, @{sleep=3000},
    @{in='observer';eval="return chrome.contextMenus.create({id:'zephium-lab',title:'Quasar lab command',contexts:['editable']})"},
    @{focus_human=$true},
    @{cdp='Input.dispatchMouseEvent';params=@{type='mousePressed';x=70;y=90;button='right';clickCount=1}},
    @{cdp='Input.dispatchMouseEvent';params=@{type='mouseReleased';x=70;y=90;button='right';clickCount=1}},
    @{sleep=500}, @{cdp='Input.dispatchKeyEvent';params=@{type='keyDown';key='Escape';code='Escape';windowsVirtualKeyCode=27}}
)
if (-not ($menu | Where-Object { $_.kind -eq 'context_menu' -and $_.value.names -contains 'extension' })) { throw 'Native extension menu missing.' }
$password = Run-Case '1password' @(
    @{human_visible=$true;navigate='$ORIGIN/form'},
    @{load=(Package 'aeblfdkhhhdcdjpifhhbdiojplfjncoa');prepare_windows=$true},
    @{view='manager'}, @{in='manager';navigate='chrome-extension://$ID/zephium-windows-host/host.html'},
    @{sleep=4000}, @{in='manager';capture_human=$true},
    @{in='manager';eval='const tabs=await chrome.tabs.query({windowId:$HUMAN_WINDOW});return {popup:await chrome.action.getPopup({tabId:tabs[0].id}),manifest:chrome.runtime.getManifest().action}';label='native-action-state'},
    @{native_action=$true;label='native-action-command'},
    @{cdp='Extensions.getExtensions';label='native-extensions-domain'},
    @{view='welcome';window=$true}, @{in='welcome';navigate='chrome-extension://$ID/app/app.html#/page/welcome?language=en'},
    @{sleep=3000}, @{in='welcome';eval='return document.body.innerText';label='welcome-document'}
)
foreach ($record in $password | Where-Object { $_.value.label }) { $record | ConvertTo-Json -Depth 12 -Compress }
Write-Output "Native sign-in and context-menu qualifiers passed. Review 1Password findings separately: $results"
