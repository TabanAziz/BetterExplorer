#Requires -Version 5.1

[CmdletBinding()]
param([ValidateNotNullOrEmpty()] [string] $TestRoot)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-common.ps1')

$paths = Get-InstallLocations $TestRoot
$lockRoot = if ($TestRoot) { $paths.Programs }
Invoke-InstallLock -TestRoot $lockRoot -Action {
    $executable = Join-Path $paths.Application 'BetterExplorer.exe'
    $bundleExecutable = Join-Path $PSScriptRoot 'BetterExplorer.exe'
    if ($bundleExecutable -ieq $executable) { $bundleExecutable = $null }
    if (-not (Test-Path -LiteralPath $paths.Application)) {
        Clear-OwnedFolderManager $executable $bundleExecutable $lockRoot
        if (-not $TestRoot -and $bundleExecutable -and [IO.File]::Exists($bundleExecutable)) {
            $null = Invoke-WinEControl $bundleExecutable '--win-e-remove-logon-for' $executable
        }
        if (-not $TestRoot -and (Test-OwnedWinERegistration $executable)) {
            Set-LegacyWinERegistration $executable $false
        }
        Write-Host 'BetterExplorer is already uninstalled.'
        return
    }
    Assert-PlainDirectory $paths.Programs
    $manifest = Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest -AllowMissingFiles
    $shortcutHash = Get-InstalledShortcutHash $paths.Shortcut $manifest.shortcut_sha256
    $previousWinE = Get-InstalledWinEState $executable $manifest $TestRoot

    $id = [guid]::NewGuid().ToString('N')
    $stage = Join-Path $paths.Programs ".BetterExplorer-remove-$id"
    $removed = [Collections.Generic.List[string]]::new()
    $cleanupStage = $true
    $winEChangeAttempted = $false
    $shortcutStage = Join-Path $stage 'BetterExplorer.lnk'
    $null = New-Item -ItemType Directory -Path $stage
    try {
        if ($previousWinE.Enabled) {
            $winEChangeAttempted = $true
            if ($previousWinE.CanControl) {
                $null = Invoke-WinEControl $executable '--win-e-off'
            } else {
                Set-LegacyWinERegistration $executable $false
            }
        }
        if (-not $TestRoot -and -not ($previousWinE.Enabled -and $previousWinE.CanControl) -and
            $bundleExecutable -and [IO.File]::Exists($bundleExecutable)) {
            $null = Invoke-WinEControl $bundleExecutable '--win-e-remove-logon-for' $executable
        }
        foreach ($name in $script:BetterExplorerPayload + $script:BetterExplorerInstallManifest) {
            $file = Join-Path $paths.Application $name
            if (-not (Test-Path -LiteralPath $file)) { continue }
            [IO.File]::Move($file, (Join-Path $stage $name))
            $removed.Add($name)
        }
        if ($shortcutHash -and (Get-InstalledShortcutHash $paths.Shortcut $shortcutHash)) {
            [IO.File]::Move($paths.Shortcut, $shortcutStage)
        }
        $stagedExecutable = Join-Path $stage 'BetterExplorer.exe'
        $controller = if ($manifest.PSObject.Properties['folder_manager_cli'] -and
            $manifest.folder_manager_cli -eq 1 -and [IO.File]::Exists($stagedExecutable)) {
            $stagedExecutable
        } else { $bundleExecutable }
        Clear-OwnedFolderManager $executable $controller $lockRoot
    } catch {
        $failure = $_
        try {
            foreach ($name in $removed) {
                [IO.File]::Move((Join-Path $stage $name), (Join-Path $paths.Application $name))
            }
            if ([IO.File]::Exists($shortcutStage)) { [IO.File]::Move($shortcutStage, $paths.Shortcut) }
        } catch {
            $cleanupStage = $false
            Write-Warning "Rollback left installation files in $stage. $($_.Exception.Message)"
        }
        if ($winEChangeAttempted) {
            try {
                Restore-InstalledWinE $executable ($previousWinE.CanControl -and $cleanupStage)
            } catch {
                Write-Warning "Could not restore the previous BetterExplorer Win+E setting. $($_.Exception.Message)"
            }
        }
        throw $failure
    } finally {
        if ($cleanupStage) {
            if ([IO.File]::Exists($shortcutStage)) { [IO.File]::Delete($shortcutStage) }
            Remove-PayloadStage $stage
        }
    }

    $remaining = @(Get-ChildItem -LiteralPath $paths.Application -Force)
    if ($remaining.Count -eq 0) {
        try { [IO.Directory]::Delete($paths.Application, $false) } catch [IO.IOException] {
            Write-Warning "Could not remove the empty installation folder $($paths.Application). $($_.Exception.Message)"
        }
    } else {
        Write-Host "Kept additional files in $($paths.Application)."
    }
    Write-Host 'Uninstalled BetterExplorer.'
}
