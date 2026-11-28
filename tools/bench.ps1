#Requires -Version 5.1

[CmdletBinding()]
param(
    [ValidateRange(1, 10000)] [int] $StartupRuns = 30,
    [ValidateRange(1, 10000)] [int] $NavigationRuns = 30,
    [ValidateRange(1, 100000)] [int] $Entries = 100000,
    [ValidateRange(0, 3600)] [int] $IdleSeconds = 0,
    [ValidateRange(5, 86400)] [int] $TimeoutSeconds = 180,
    [string] $BinaryPath,
    [switch] $CompareEnumeration
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Start-BenchmarkApp {
    param([string] $InitialPath, [string] $ReportPath, [string] $TargetPath, [int] $Runs)

    $environment = @{
        BETTEREXPLORER_EPHEMERAL = '1'
        BETTEREXPLORER_BENCH_NOACTIVATE = '1'
        BETTEREXPLORER_BENCH_REPORT = $ReportPath
        BETTEREXPLORER_BENCH_TARGET = $TargetPath
        BETTEREXPLORER_BENCH_RUNS = [string] $Runs
    }
    $previous = @{}
    try {
        foreach ($name in $environment.Keys) {
            $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            if ($environment[$name] -eq '') {
                [Environment]::SetEnvironmentVariable($name, [NullString]::Value, 'Process')
            } else {
                [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process')
            }
        }
        $quotedPath = '"{0}"' -f $InitialPath
        Start-Process -FilePath $binary -ArgumentList $quotedPath -WorkingDirectory $repo -PassThru
    } finally {
        foreach ($name in $previous.Keys) {
            if ($null -eq $previous[$name]) {
                [Environment]::SetEnvironmentVariable($name, [NullString]::Value, 'Process')
            } else {
                [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
            }
        }
    }
}

function Stop-BenchmarkApp {
    param([Diagnostics.Process] $App)

    try {
        if ($App.HasExited) { return }
        if ($App.CloseMainWindow() -and $App.WaitForExit(3000)) { return }
        if (-not $App.HasExited) {
            try { $App.Kill() } catch [InvalidOperationException] {
                if (-not $App.HasExited) { throw }
            }
            if (-not $App.WaitForExit(5000)) { throw "Could not stop benchmark process $($App.Id)." }
        }
    } finally {
        $App.Dispose()
    }
}

function Invoke-AppReport {
    param([string] $Name, [int] $Runs, [string] $TargetPath = '')

    $reportPath = Join-Path $outputPath "$Name.json"
    $app = Start-BenchmarkApp -InitialPath $smallPath -ReportPath $reportPath -TargetPath $TargetPath -Runs $Runs
    try {
        if (-not $app.WaitForExit($TimeoutSeconds * 1000)) { throw "$Name timed out after $TimeoutSeconds seconds." }
        if ($app.ExitCode -ne 0) { throw "$Name exited with code $($app.ExitCode)." }
        if (-not [IO.File]::Exists($reportPath)) { throw "$Name did not write $reportPath." }

        $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        if (@($report.navigation).Count -ne $Runs) { throw "$Name did not report $Runs navigation samples." }
        $report
    } finally {
        Stop-BenchmarkApp -App $app
    }
}

function Read-Milliseconds {
    param([object] $Value, [string] $Metric)

    if ($null -eq $Value -or $Value -is [string] -or $Value -is [bool] -or $Value -isnot [ValueType]) {
        throw "Invalid timing for $Metric."
    }
    $milliseconds = [double] $Value
    if ($milliseconds -lt 0 -or [double]::IsNaN($milliseconds) -or [double]::IsInfinity($milliseconds)) {
        throw "Invalid timing for $Metric."
    }
    $milliseconds
}

function Get-Percentiles {
    param([double[]] $Values)

    $ordered = @($Values | Sort-Object)
    [ordered] @{
        count = $ordered.Count
        p50 = [math]::Round($ordered[[math]::Ceiling($ordered.Count * 0.50) - 1], 3)
        p95 = [math]::Round($ordered[[math]::Ceiling($ordered.Count * 0.95) - 1], 3)
        p99 = [math]::Round($ordered[[math]::Ceiling($ordered.Count * 0.99) - 1], 3)
    }
}

function Measure-AppIdle {
    $app = Start-BenchmarkApp -InitialPath $smallPath -ReportPath '' -TargetPath '' -Runs 0
    try {
        $readiness = [Diagnostics.Stopwatch]::StartNew()
        while ($app.MainWindowHandle -eq [IntPtr]::Zero) {
            if ($app.HasExited) { throw 'The idle process exited before opening a window.' }
            if ($readiness.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'The idle process did not open a window.' }
            Start-Sleep -Milliseconds 100
            $app.Refresh()
        }

        $settleSeconds = 5
        Start-Sleep -Seconds $settleSeconds
        $app.Refresh()
        $cpuBefore = $app.TotalProcessorTime.TotalMilliseconds
        $elapsed = [Diagnostics.Stopwatch]::StartNew()
        if ($app.WaitForExit($IdleSeconds * 1000)) { throw 'The idle process exited during the sample.' }
        $app.Refresh()
        $cpuMilliseconds = $app.TotalProcessorTime.TotalMilliseconds - $cpuBefore
        $elapsed.Stop()
        [ordered] @{
            requested_seconds = $IdleSeconds
            settle_seconds = $settleSeconds
            elapsed_seconds = [math]::Round($elapsed.Elapsed.TotalSeconds, 3)
            process_cpu_ms = [math]::Round($cpuMilliseconds, 3)
            percent_one_logical_core = [math]::Round(100 * $cpuMilliseconds / $elapsed.Elapsed.TotalMilliseconds, 6)
        }
    } finally {
        Stop-BenchmarkApp -App $app
    }
}

if (-not [Environment]::UserInteractive -or (Get-Process -Id $PID).SessionId -eq 0) {
    throw 'Run this benchmark from an interactive Windows desktop session.'
}

$repo = Split-Path -Parent $PSScriptRoot
$binary = if ($BinaryPath) { $BinaryPath } else { Join-Path $repo 'target/release/BetterExplorer.exe' }
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build the release or pass an existing BetterExplorer.exe with -BinaryPath.' }
$binary = (Resolve-Path -LiteralPath $binary).ProviderPath

$runId = [guid]::NewGuid().ToString('N')
$startedAt = [DateTime]::UtcNow
$timestamp = $startedAt.ToString('yyyyMMdd-HHmmss', [Globalization.CultureInfo]::InvariantCulture)
$outputPath = Join-Path $repo "bench/out/$timestamp-$runId"
$null = New-Item -ItemType Directory -Path $outputPath

$temp = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).ProviderPath
$fixtureName = "BetterExplorer-bench-$runId"
$fixturePath = [IO.Path]::GetFullPath((Join-Path $temp $fixtureName))
$smallPath = Join-Path $fixturePath 'Small directory'
$largePath = Join-Path $fixturePath 'Large directory'
$fixtureDrive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($fixturePath))
if ($fixtureDrive.DriveType -ne [IO.DriveType]::Fixed) { throw 'The benchmark requires a local fixed drive for the temp directory.' }

if (-not ('BetterExplorerBenchmarkFixture' -as [type])) {
    Add-Type -TypeDefinition @'
using System.Globalization;
using System.IO;

public static class BetterExplorerBenchmarkFixture
{
    public static void Create(string path, int count)
    {
        Directory.CreateDirectory(path);
        for (int index = 1; index <= count; index++)
        {
            string name = "entry-" + index.ToString("D6", CultureInfo.InvariantCulture) + ".txt";
            using (new FileStream(Path.Combine(path, name), FileMode.CreateNew)) { }
        }
    }
}
'@
}

$null = New-Item -ItemType Directory -Path $fixturePath
try {
    Write-Host "Creating $Entries benchmark files on $($fixtureDrive.Name)..."
    [BetterExplorerBenchmarkFixture]::Create($smallPath, 16)
    [BetterExplorerBenchmarkFixture]::Create($largePath, $Entries)

    if ($CompareEnumeration) {
        $env:BETTEREXPLORER_BENCH_PROBE = '1'
        $arms = @{ bulk = @(); standard = @(); native = @() }
        $order = @('native', 'bulk', 'standard', 'standard', 'native', 'bulk', 'bulk', 'standard', 'native')
        try {
            for ($i = 0; $i -lt $order.Count; $i++) {
                $arm = $order[$i]
                if ($arm -eq 'standard') { $env:BETTEREXPLORER_BENCH_STANDARD = '1' }
                else { Remove-Item Env:BETTEREXPLORER_BENCH_STANDARD -ErrorAction SilentlyContinue }
                if ($arm -eq 'native') { $env:BETTEREXPLORER_BENCH_NATIVE_DEFAULT = '1' }
                else { Remove-Item Env:BETTEREXPLORER_BENCH_NATIVE_DEFAULT -ErrorAction SilentlyContinue }
                $report = Invoke-AppReport -Name "probe-$i-$arm" -Runs $NavigationRuns -TargetPath $largePath
                foreach ($sample in $report.navigation) {
                    if ($sample.rows -ne $Entries) { throw "Wrong row count for $arm." }
                    $stages = @($sample.first_row_stages_ms)
                    $points = @(
                        (Read-Milliseconds $stages[0] 'open called')
                        (Read-Milliseconds $stages[1] 'open returned')
                        (Read-Milliseconds $stages[2] 'first match')
                        (Read-Milliseconds $stages[3] 'first batch post')
                        (Read-Milliseconds $stages[4] 'first batch delivered')
                        (Read-Milliseconds $sample.first_ms 'first paint')
                        (Read-Milliseconds $sample.complete_ms 'complete paint')
                    )
                    for ($j = 1; $j -lt $points.Count; $j++) {
                        if ($points[$j] -lt $points[$j-1]) { throw "Out-of-order stage timings for $arm." }
                    }
                    $arms[$arm] += [pscustomobject] @{
                        open_ms = $points[1] - $points[0]
                        next_match_ms = $points[2] - $points[1]
                        row_build_ms = $points[3] - $points[2]
                        delivery_ms = $points[4] - $points[3]
                        delivery_to_paint_ms = $points[5] - $points[4]
                        first_ms = $points[5]
                        complete_ms = $points[6]
                    }
                }
            }
        } finally {
            Remove-Item Env:BETTEREXPLORER_BENCH_PROBE -ErrorAction SilentlyContinue
            Remove-Item Env:BETTEREXPLORER_BENCH_NATIVE_DEFAULT -ErrorAction SilentlyContinue
            Remove-Item Env:BETTEREXPLORER_BENCH_STANDARD -ErrorAction SilentlyContinue
        }
        $comparison = [ordered] @{ entries = $Entries; order = $order; method = 'Shared warm fixture, three processes per arm, one warm-up navigation per process.' }
        foreach ($arm in @('bulk', 'standard', 'native')) {
            $comparison[$arm] = [ordered] @{}
            foreach ($metric in $arms[$arm][0].PSObject.Properties.Name) {
                $comparison[$arm][$metric] = Get-Percentiles -Values ($arms[$arm] | ForEach-Object { $_.$metric })
            }
        }
        $json = $comparison | ConvertTo-Json -Depth 6
        [IO.File]::WriteAllText((Join-Path $outputPath 'enumeration.json'), $json)
        Write-Output $json
        return
    }

    Write-Host "Measuring $StartupRuns warm launches after one unmeasured launch..."
    $warmup = Invoke-AppReport -Name 'startup-warmup' -Runs 0
    $null = Read-Milliseconds $warmup.startup_ms 'startup warm-up'
    $startup = foreach ($index in 1..$StartupRuns) {
        $report = Invoke-AppReport -Name ('startup-{0:D3}' -f $index) -Runs 0
        Read-Milliseconds $report.startup_ms "startup $index"
    }

    Write-Host "Measuring $NavigationRuns warm navigations in one process..."
    $navigation = Invoke-AppReport -Name 'navigation' -Runs $NavigationRuns -TargetPath $largePath
    foreach ($sample in $navigation.navigation) {
        if ($sample.rows -ne $Entries) { throw "Navigation reported $($sample.rows) rows. Expected $Entries." }
    }
    $first = foreach ($sample in $navigation.navigation) { Read-Milliseconds $sample.first_ms 'first rows' }
    $complete = foreach ($sample in $navigation.navigation) { Read-Milliseconds $sample.complete_ms 'complete listing' }
    $dispatch = foreach ($sample in $navigation.navigation) { Read-Milliseconds $sample.max_dispatch_ms 'maximum dispatch' }

    $idle = $null
    if ($IdleSeconds -gt 0) {
        Write-Host "Measuring idle CPU for $IdleSeconds seconds. Leave the benchmark window untouched."
        $idle = Measure-AppIdle
    }

    $windows = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $processor = Get-ItemPropertyValue -LiteralPath 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0' -Name ProcessorNameString
    $summary = [ordered] @{
        started_utc = $startedAt.ToString('O')
        environment = [ordered] @{
            windows_version = $windows.DisplayVersion
            windows_build = "$($windows.CurrentBuildNumber).$($windows.UBR)"
            processor = $processor.Trim()
            logical_processors = [Environment]::ProcessorCount
            ui_culture = [Globalization.CultureInfo]::CurrentUICulture.Name
            powershell = $PSVersionTable.PSVersion.ToString()
            fixture_drive = $fixtureDrive.Name
            fixture_file_system = $fixtureDrive.DriveFormat
            binary_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        }
        method = [ordered] @{
            cache = 'Warm Windows cache. One unmeasured startup process and one in-process navigation warm-up.'
            startup = 'Time to the first DWM-flushed paint from a dispatched window message. One process per sample.'
            navigation = 'App-internal timings in one visible process. Dispatch reports the longest sampled message dispatch for each navigation.'
            measurement_limits = 'App timings do not independently verify desktop composition, queued message delivery or real input latency. Windows scheduling and background activity affect the samples.'
            idle = 'Process CPU time divided by elapsed time, expressed as a percent of one logical core.'
            percentile = 'Nearest rank.'
            small_entries = 16
            large_entries = $Entries
            fixture = 'Zero-length text files in local temporary directories.'
        }
        startup_ms = Get-Percentiles -Values $startup
        navigation_first_ms = Get-Percentiles -Values $first
        navigation_complete_ms = Get-Percentiles -Values $complete
        navigation_max_dispatch_ms = Get-Percentiles -Values $dispatch
        idle = $idle
    }
    $summaryJson = $summary | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $outputPath 'summary.json'), $summaryJson)
    Write-Output $summaryJson
    Write-Host "Raw samples and summary: $outputPath"
} finally {
    $fixture = Get-Item -LiteralPath $fixturePath -Force
    if ($fixture.Name -ne $fixtureName -or $fixture.Parent.FullName.TrimEnd('\') -ne $temp.TrimEnd('\') -or
        ($fixture.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing to remove unexpected fixture path $fixturePath."
    }
    [IO.Directory]::Delete($fixture.FullName, $true)
}
