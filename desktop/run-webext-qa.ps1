param([switch]$Build, [switch]$Release,
    [ValidatePattern('^[a-zA-Z0-9-]{1,48}$')][string]$Session)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$qaDir = Join-Path $repo 'target\webext-qa'
$qaExe = Join-Path $qaDir 'Quasar Extensions QA.exe'
$buildProfile = if ($Release) { 'release' } else { 'debug' }
$developmentExe = Join-Path $repo "target\$buildProfile\zephium-desktop.exe"
$sourceExecutables = @('debug', 'release') | ForEach-Object { Join-Path $repo "target\$_\zephium-desktop.exe" }
$running = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
    -not $_.HasExited -and ($_.Path -eq $qaExe -or $_.Path -in $sourceExecutables)
})
if ($running.Count) { throw 'Close this checkout''s QA/development app before building or launching QA.' }
if ($Build) {
    # Prefer this checkout's pinned tools when present; ordinary PATH also works.
    foreach ($relative in @('target\windows-tools\pnpm\node_modules\.bin', 'target\windows-tools\node-v24.18.0-win-x64')) {
        $toolPath = Join-Path $repo $relative
        if (Test-Path -LiteralPath $toolPath) { $env:PATH = $toolPath + ';' + $env:PATH }
    }
    Push-Location -LiteralPath $repo
    try {
        $buildArguments = @('-C', 'desktop', 'exec', 'tauri', 'build', '--no-bundle', '--features', 'webext-qa', '--config', 'tauri.webext-qa.conf.json')
        if (-not $Release) { $buildArguments += '--debug' }
        & pnpm.cmd @buildArguments
        if ($LASTEXITCODE -ne 0) { throw 'QA build failed.' }
        $metadata = (Get-Item -LiteralPath $developmentExe).VersionInfo
        if ($metadata.ProductName -ne 'Quasar Extensions QA') { throw 'Refusing a binary without the QA identity.' }
        New-Item -ItemType Directory -Path $qaDir -Force | Out-Null
        Copy-Item -LiteralPath $developmentExe -Destination $qaExe -Force
        & git rev-parse HEAD | Set-Content -LiteralPath (Join-Path $qaDir 'revision.txt')
        $buildProfile | Set-Content -LiteralPath (Join-Path $qaDir 'build-profile.txt')
    } finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $qaExe)) { throw 'Run this script with -Build first.' }
if ((Get-Item -LiteralPath $qaExe).VersionInfo.ProductName -ne 'Quasar Extensions QA') {
    throw 'Refusing a binary without the QA identity.'
}
$launch = @{FilePath=$qaExe;WorkingDirectory=$repo;WindowStyle='Normal';PassThru=$true;RedirectStandardOutput=(Join-Path $qaDir 'stdout.log');RedirectStandardError=(Join-Path $qaDir 'stderr.log')}
if ($Session) { $launch.ArgumentList = @("--webext-qa-session=$Session") }
$process = Start-Process @launch
$processHandle = $process.Handle
Start-Sleep -Seconds 2
if ($process.HasExited) {
    Get-Content -LiteralPath (Join-Path $qaDir 'stderr.log') -Tail 20
    throw "QA exited during startup (code $($process.ExitCode))."
}
Write-Output "Started Quasar Extensions QA (PID $($process.Id))."
if ($Session) { Write-Output "Data: %APPDATA%\app.quasar.webext-qa\qa-sessions\session-$Session" }
else { Write-Output 'Data: %APPDATA%\app.quasar.webext-qa' }
