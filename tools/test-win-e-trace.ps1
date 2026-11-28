param([Parameter(Mandatory = $true)][string]$BinaryPath)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $BinaryPath).Path
$dir = Join-Path $env:LOCALAPPDATA 'BetterExplorerWinETrace'
if (Test-Path -LiteralPath $dir) { throw 'Expected a clean Windows trace directory.' }
function Set-ListenerState([string]$Mode) {
    $control = Start-Process -FilePath $exe -ArgumentList $Mode -PassThru
    if (-not $control.WaitForExit(10000) -or $control.ExitCode -ne 0) {
        throw "Listener control failed: $Mode"
    }
}
function Wait-Phase([string]$Filter, [string]$Phase) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 10000) {
        foreach ($file in Get-ChildItem -LiteralPath $dir -Filter $Filter -File -ErrorAction SilentlyContinue) {
            $lines = @(Get-Content -LiteralPath $file.FullName)
            if ($lines.Count -ge 3 -and $lines[0] -match '^# qpc_frequency_hz=[1-9][0-9]*$') {
                $events = @($lines | Select-Object -Skip 1 | ConvertFrom-Csv -Delimiter "`t")
                if ($events.phase -contains $Phase) { return $events }
            }
        }
        Start-Sleep -Milliseconds 50
    }
    throw "Missing trace marker: $Phase."
}
$app = $null
try {
    Set-ListenerState '--win-e-on-setup'
    $ready = @(Wait-Phase 'listener-*.tsv' 'listener_ready' | Where-Object phase -eq listener_ready)
    if ($ready.Count -ne 1 -or [long]$ready[0].qpc -le 0) { throw 'Invalid listener readiness marker.' }
    $env:BETTEREXPLORER_BENCH_NOACTIVATE = '1'
    $app = Start-Process -FilePath $exe -PassThru
    $window = @(Wait-Phase "window-$($app.Id)-*.tsv" 'window_shown')
    if ($window.Count -ne 2 -or $window[0].phase -ne 'ui_entered' -or
        [int]$window[1].value -ne $app.Id -or [long]$window[1].qpc -le [long]$window[0].qpc) {
        throw 'Window markers do not match the process or chronological order.'
    }
    Write-Host 'Observed listener-ready and actual app-window trace records. Did not inject or time Win+E.'
} finally {
    if ($app -and -not $app.HasExited) { $null = $app.CloseMainWindow() }
    Set-ListenerState '--win-e-off'
}
