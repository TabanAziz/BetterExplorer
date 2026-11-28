#Requires -Version 5.1
param(
    [Parameter(Mandatory)][string] $Previous,
    [Parameter(Mandatory)][string] $Draft
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or -not [Environment]::UserInteractive) {
    throw 'This diagnostic requires a disposable interactive GitHub-hosted Windows session.'
}
if (Get-ChildItem Env:BETTEREXPLORER_BENCH_* -ErrorAction SilentlyContinue) {
    throw 'Benchmark mode must not be enabled.'
}
$programs = Join-Path $env:LOCALAPPDATA 'Programs'
$exe = Join-Path $programs 'BetterExplorer\BetterExplorer.exe'
$uninstaller = Join-Path $programs 'BetterExplorer Setup\unins000.exe'
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$taskName = "BetterExplorer Win+E Logon $sid"
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if ((Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $uninstaller) -or
    (Get-ItemProperty -LiteralPath $runKey -Name BetterExplorerWinE -ErrorAction SilentlyContinue) -or
    (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw 'This diagnostic requires a clean account.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class InstalledExplorerProbe {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    private static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string name, string caption);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    public static IntPtr MainFor(uint target) {
        IntPtr window = IntPtr.Zero;
        while ((window = FindWindowEx(IntPtr.Zero, window, "BetterExplorerWindow", null)) != IntPtr.Zero) {
            uint pid;
            GetWindowThreadProcessId(window, out pid);
            if (pid == target && IsWindowVisible(window)) return window;
        }
        return IntPtr.Zero;
    }
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    private static extern IntPtr SendMessageTimeout(IntPtr window, uint msg, UIntPtr wparam,
        IntPtr lparam, uint flags, uint timeout, out UIntPtr result);
    public static bool HelperReady(string exe) {
        return FindWindowEx(new IntPtr(-3), IntPtr.Zero, "BetterExplorerWinEListener", exe) != IntPtr.Zero;
    }
    public static int Rows(IntPtr main) {
        if (main == IntPtr.Zero || !IsWindowVisible(main)) return -2;
        IntPtr list = FindWindowEx(main, IntPtr.Zero, "SysListView32", null);
        if (list == IntPtr.Zero || !IsWindowVisible(list)) return -1;
        UIntPtr count;
        if (SendMessageTimeout(list, 0x1004, UIntPtr.Zero, IntPtr.Zero, 3, 25, out count) == IntPtr.Zero)
            return -1;
        return checked((int)count.ToUInt32());
    }
}
'@
function Invoke-CheckedProcess([string] $Path, [string] $Arguments, [int] $Timeout) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
    try {
        if (-not $process.WaitForExit($Timeout)) {
            $process.Kill()
            throw "Timed out: $Path $Arguments"
        }
        if ($process.ExitCode -ne 0) { throw "Exit $($process.ExitCode): $Path $Arguments" }
    } finally {
        $process.Dispose()
    }
}
function Measure-Helper([int] $ExpectedPriority) {
    $task = Get-ScheduledTask -TaskPath '\' -TaskName $taskName
    if ($task.Settings.Priority -ne $ExpectedPriority -or $task.Principal.LogonType -ne 'Interactive' -or
        $task.Actions.Count -ne 1 -or $task.Actions[0].Execute.Trim('"') -ine $exe -or
        $task.Actions[0].Arguments -ne '--win-e-listener' -or $task.Triggers.Count -ne 1 -or
        $task.Triggers[0].CimClass.CimClassName -ne 'MSFT_TaskLogonTrigger') {
        throw "Unexpected installed task configuration (expected priority $ExpectedPriority)."
    }
    Invoke-CheckedProcess $exe '--win-e-pause' 10000
    if ([InstalledExplorerProbe]::HelperReady($exe)) { throw 'Win+E helper did not pause.' }
    $stopWait = [Diagnostics.Stopwatch]::StartNew()
    while ((Get-ScheduledTask -TaskName $taskName).State -eq 'Running') {
        if ($stopWait.ElapsedMilliseconds -ge 8000) { throw 'Previous task instance did not exit.' }
        Start-Sleep -Milliseconds 50
    }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Start-ScheduledTask -InputObject $task
    while (-not [InstalledExplorerProbe]::HelperReady($exe)) {
        if ($clock.ElapsedMilliseconds -ge 10000) { throw 'Installed shortcut helper not ready.' }
        Start-Sleep -Milliseconds 10
    }
    $clock.ElapsedMilliseconds
}
function Measure-Installed([string] $Version, [int] $Priority, [int] $FirstFolder) {
    $helperTimes = @(1..5 | ForEach-Object { Measure-Helper $Priority })
    Write-Host "BEX_HELPER $Version priority=$Priority samples_ms=$($helperTimes -join ',')"
    # The default Details view inserts one date group header once the scan completes.
    $external = @(0..3 | ForEach-Object { Measure-FolderLaunch $folders[($FirstFolder + $_)] ($count + 1) })
    [ordered]@{
        install = $Version; executable_sha256 = (Get-FileHash -LiteralPath $exe).Hash; task_priority = $Priority
        helper_ready_ms = $helperTimes; folder_launch = $external
    }
}
function Measure-FolderLaunch([string] $Folder, [int] $ExpectedRows) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -ArgumentList ('"{0}"' -f $Folder) -PassThru
    try {
        $visible = $null
        $first = $null
        $last = -2
        $highest = -2
        while ($clock.ElapsedMilliseconds -lt 12000) {
            $process.Refresh()
            if ($process.HasExited) { throw 'Installed app exited before loading the folder.' }
            $main = [InstalledExplorerProbe]::MainFor([uint32]$process.Id)
            $rows = [InstalledExplorerProbe]::Rows($main)
            $last = $rows
            $highest = [Math]::Max($highest, $rows)
            $elapsed = $clock.ElapsedMilliseconds
            if ($null -eq $visible -and $rows -ge -1) { $visible = $elapsed }
            if ($null -eq $first -and $rows -gt 1) { $first = $elapsed }
            if ($rows -eq $ExpectedRows) {
                return [ordered]@{ visible_window_ms = $visible; first_rows_ms = $first; full_rows_ms = $elapsed; rows = $rows }
            }
            Start-Sleep -Milliseconds 10
        }
        throw "External list probe could not confirm $ExpectedRows rows (last=$last, max=$highest, title=$($process.MainWindowTitle))."
    } finally {
        if (-not $process.HasExited) {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(4000)) {
                $process.Kill()
                $null = $process.WaitForExit(4000)
                throw 'The installed window did not close normally.'
            }
        }
        $process.Dispose()
    }
}
$fixture = Join-Path $env:RUNNER_TEMP ('bex-installed-' + [guid]::NewGuid().ToString('N'))
$count = 2000
$modified = [DateTime]::UtcNow.Date.AddHours(-24)
$folders = foreach ($i in 1..8) {
    $folder = Join-Path $fixture "folder $i"
    $null = [IO.Directory]::CreateDirectory($folder)
    for ($n = 0; $n -lt $count; $n++) {
        $file = Join-Path $folder ("item-{0:d5}.txt" -f $n)
        [IO.File]::WriteAllText($file, 'fixture')
        [IO.File]::SetLastWriteTimeUtc($file, $modified)
    }
    $folder
}
try {
    Invoke-CheckedProcess $Previous '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-' 90000
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Baseline installation missing.' }
    $previousHash = (Get-FileHash -LiteralPath $exe).Hash
    Invoke-CheckedProcess $exe '--win-e-on' 15000
    $oldTask = Get-ScheduledTask -TaskPath '\' -TaskName $taskName
    $oldPriority = [int]$oldTask.Settings.Priority
    $baseline = Measure-Installed '0.1.10' $oldPriority 0
    Write-Host ('BEX_BASELINE ' + ($baseline | ConvertTo-Json -Depth 5 -Compress))
    Invoke-CheckedProcess $Draft '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-' 90000
    $draftHash = (Get-FileHash -LiteralPath $exe).Hash
    if ($draftHash -eq $previousHash) { throw 'The installer did not replace the installed executable.' }
    Invoke-CheckedProcess $exe '--win-e-enabled' 10000
    $candidate = Measure-Installed '0.1.11' 5 4
    Write-Host ('BEX_CANDIDATE ' + ($candidate | ConvertTo-Json -Depth 5 -Compress))
    Write-Host 'Installed compare only: on-demand scheduled task, no sign-in/reboot or physical Win+E. Four ordinary launches and five task samples each on Windows Server.'
} finally {
    if (Test-Path -LiteralPath $uninstaller) {
        Invoke-CheckedProcess $uninstaller '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' 90000
        if ((Test-Path -LiteralPath $exe) -or (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
            throw 'Uninstall left the executable or the Win+E task behind.'
        }
    }
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}
