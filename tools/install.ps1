#Requires -Version 5.1

[CmdletBinding()]
param(
    [switch] $StartMenuShortcut,
    [switch] $Quiet,
    [switch] $QueryOptions,
    [switch] $ValidateOnly,
    [ValidateSet('WindowsExplorer', 'BetterExplorer')]
    [string] $WinE,
    [ValidateSet('WindowsExplorer', 'BetterExplorer')]
    [string] $FolderManager,
    [ValidateNotNullOrEmpty()]
    [string] $TestRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-common.ps1')

$install = {
    if (Test-Path -LiteralPath $paths.Programs) { Assert-PlainDirectory $paths.Programs }

    $previous = $null
    $shortcutHash = $null
    if (Test-Path -LiteralPath $paths.Application) {
        $previous = Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest
        $managed = $script:BetterExplorerPayload + $script:BetterExplorerInstallManifest
        $unmanaged = @(Get-ChildItem -LiteralPath $paths.Application -Force | Where-Object Name -notin $managed)
        if ($unmanaged.Count -gt 0) { throw "The installation folder contains additional files. Keep them elsewhere before updating $($paths.Application)." }
        $shortcutHash = Get-InstalledShortcutHash $paths.Shortcut $previous.shortcut_sha256
    }

    $executable = Join-Path $paths.Application 'BetterExplorer.exe'
    $previousWinE = Get-InstalledWinEState $executable $previous $TestRoot
    $enableWinE = $WinE -eq 'BetterExplorer' -or (-not $WinE -and $previous -and $previousWinE.Enabled)
    if (-not $TestRoot -and $WinE -and (Test-OtherWinERegistration $executable)) {
        $message = 'Win+E is managed by another copy of BetterExplorer. Turn it off in that copy to change it here.'
        if ($enableWinE) { throw $message }
        Write-Warning $message
    }
    $createShortcut = $addShortcut -and -not $shortcutHash
    if ($createShortcut -and (Test-Path -LiteralPath $paths.Shortcut)) {
        throw "A separate item already occupies the Start Menu shortcut path $($paths.Shortcut)."
    }
    if ($createShortcut) { Assert-PlainDirectory (Split-Path -Parent $paths.Shortcut) }
    if ($ValidateOnly) { return }

    $null = [IO.Directory]::CreateDirectory($paths.Programs)
    Assert-PlainDirectory $paths.Programs

    $id = [guid]::NewGuid().ToString('N')
    $stage = Join-Path $paths.Programs ".BetterExplorer-new-$id"
    $backup = Join-Path $paths.Programs ".BetterExplorer-old-$id"
    $shortcutStage = $null
    $oldMoved = $false
    $newMoved = $false
    $shortcutMoved = $false
    $winEPauseAttempted = $false
    $winEChangeAttempted = $false
    $completed = $false
    $cleanupStage = $true
    $null = New-Item -ItemType Directory -Path $stage
    try {
        foreach ($name in $script:BetterExplorerPayload) {
            $destination = Join-Path $stage $name
            [IO.File]::Copy((Join-Path $PSScriptRoot $name), $destination)
            if ((Get-OwnedFileHash $destination) -cne $bundle.files.PSObject.Properties[$name].Value) {
                throw "The staged copy of $name did not match the package checksum."
            }
        }

        if ($createShortcut) {
            $shortcutDirectory = Split-Path -Parent $paths.Shortcut
            $shortcutStage = Join-Path $shortcutDirectory ".BetterExplorer-new-$id.lnk"
            $shell = New-Object -ComObject WScript.Shell
            $link = $shell.CreateShortcut($shortcutStage)
            $link.TargetPath = Join-Path $paths.Application 'BetterExplorer.exe'
            $link.WorkingDirectory = $paths.Application
            $link.Description = 'BetterExplorer'
            $link.Save()
            $shortcutHash = Get-OwnedFileHash $shortcutStage
        }
        $manifest = [ordered] @{
            product = 'BetterExplorer'
            schema = 1
            version = $bundle.version
            files = $bundle.files
            shortcut_sha256 = $shortcutHash
            win_e_cli = 1
            folder_manager_cli = 1
        }
        Write-PayloadManifest $manifest (Join-Path $stage $script:BetterExplorerInstallManifest)

        if ($previousWinE.Enabled -and $previousWinE.CanControl) {
            $winEPauseAttempted = $true
            $null = Invoke-WinEControl $executable '--win-e-pause'
        }
        if ($previous) {
            [IO.Directory]::Move($paths.Application, $backup)
            $oldMoved = $true
        }
        [IO.Directory]::Move($stage, $paths.Application)
        $newMoved = $true
        if (-not $TestRoot -and ($enableWinE -or $previousWinE.Enabled)) {
            $command = if (-not $enableWinE) { '--win-e-off' }
                elseif ($winEPauseAttempted -and -not $WinE) { '--win-e-resume' }
                else { '--win-e-on-setup' }
            $winEChangeAttempted = $true
            $null = Invoke-WinEControl $executable $command
        }
        if ($createShortcut) {
            [IO.File]::Move($shortcutStage, $paths.Shortcut)
            $shortcutMoved = $true
        }
        if ($FolderManager -or $previous) {
            $command = switch ($FolderManager) {
                'BetterExplorer' { '--folder-manager-on' }
                'WindowsExplorer' { '--folder-manager-off' }
                default { '--folder-manager-upgrade' }
            }
            $null = Invoke-FolderManagerControl $executable $executable $command $lockRoot
        }
        $completed = $true
    } catch {
        $failure = $_
        if ($winEChangeAttempted) {
            try {
                $command = if ($previousWinE.Enabled) { '--win-e-pause' } else { '--win-e-off' }
                $null = Invoke-WinEControl $executable $command
            } catch {
                Write-Warning "Could not stop the new BetterExplorer Win+E listener. $($_.Exception.Message)"
            }
        }
        try {
            if ($shortcutMoved -and (Get-InstalledShortcutHash $paths.Shortcut $shortcutHash)) {
                [IO.File]::Delete($paths.Shortcut)
            }
            if ($newMoved) { [IO.Directory]::Move($paths.Application, $stage) }
            if ($oldMoved) { [IO.Directory]::Move($backup, $paths.Application) }
        } catch {
            $cleanupStage = $false
            Write-Warning "Rollback left files in $stage or $backup. $($_.Exception.Message)"
        }
        if ($previousWinE.Enabled -and ($winEPauseAttempted -or $winEChangeAttempted)) {
            try {
                Restore-InstalledWinE $executable ($previousWinE.CanControl -and $cleanupStage)
            } catch {
                Write-Warning "Could not restore the previous BetterExplorer Win+E setting. $($_.Exception.Message)"
            }
        }
        throw $failure
    } finally {
        if ($cleanupStage) { Remove-PayloadStage $stage }
        if ($completed -and $oldMoved) { Remove-PayloadStage $backup }
        if ($shortcutStage -and [IO.File]::Exists($shortcutStage)) { [IO.File]::Delete($shortcutStage) }
    }

    if (-not $TestRoot -and $enableWinE) {
        try {
            $logonCommand = if ($WinE -eq 'BetterExplorer') { '--win-e-logon' }
                else { '--win-e-logon-upgrade' }
            $null = Invoke-WinEControl $executable $logonCommand
        } catch {
            Write-Warning 'Could not register the Win+E sign-in task. Open BetterExplorer Settings and select BetterExplorer to retry.'
        }
    }
    Write-Host "Installed BetterExplorer $($bundle.version) in $($paths.Application)."
}

$paths = Get-InstallLocations $TestRoot
$bundle = Read-PayloadManifest -Directory $PSScriptRoot -Name 'bundle-manifest.json'
$lockRoot = if ($TestRoot) { $paths.Programs }

if ($QueryOptions) {
    $installedExecutable = Join-Path $paths.Application 'BetterExplorer.exe'
    $bundleExecutable = Join-Path $PSScriptRoot 'BetterExplorer.exe'
    $installed = if (Test-Path -LiteralPath $paths.Application) {
        Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest
    }
    $winEState = if (-not $TestRoot -and (Test-OtherWinERegistration $installedExecutable)) { 'other' }
        elseif ((Get-InstalledWinEState $installedExecutable $installed $TestRoot).Enabled) { 'enabled' }
        else { 'available' }
    $folderState = Invoke-FolderManagerControl $bundleExecutable $installedExecutable '--folder-manager-state' $lockRoot
    $shortcutState = if ($installed -and (Get-InstalledShortcutHash $paths.Shortcut $installed.shortcut_sha256)) { 'installed' }
        elseif (Test-Path -LiteralPath $paths.Shortcut) { 'other' }
        else { 'available' }
    Write-Output $winEState, $folderState, $shortcutState
    return
}

$addShortcut = $StartMenuShortcut.IsPresent
Invoke-InstallLock $install -TestRoot $lockRoot
