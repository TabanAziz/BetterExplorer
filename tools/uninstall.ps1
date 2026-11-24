#Requires -Version 5.1

[CmdletBinding()]
param([string] $TestRoot)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-common.ps1')

$paths = Get-InstallLocations $TestRoot
Invoke-InstallLock {
    if (-not (Test-Path -LiteralPath $paths.Application)) {
        Write-Host 'BetterExplorer is already uninstalled.'
        return
    }
    Assert-PlainDirectory $paths.Programs
    $manifest = Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest -AllowMissingFiles
    $shortcutHash = Get-InstalledShortcutHash $paths.Shortcut $manifest.shortcut_sha256

    $id = [guid]::NewGuid().ToString('N')
    $stage = Join-Path $paths.Programs ".BetterExplorer-remove-$id"
    $removed = [Collections.Generic.List[string]]::new()
    $cleanupStage = $true
    $null = New-Item -ItemType Directory -Path $stage
    try {
        foreach ($name in $script:BetterExplorerPayload + $script:BetterExplorerInstallManifest) {
            $file = Join-Path $paths.Application $name
            if (-not (Test-Path -LiteralPath $file)) { continue }
            [IO.File]::Move($file, (Join-Path $stage $name))
            $removed.Add($name)
        }
        if ($shortcutHash -and (Get-InstalledShortcutHash $paths.Shortcut $shortcutHash)) {
            [IO.File]::Delete($paths.Shortcut)
        }
    } catch {
        $failure = $_
        try {
            foreach ($name in $removed) {
                [IO.File]::Move((Join-Path $stage $name), (Join-Path $paths.Application $name))
            }
        } catch {
            $cleanupStage = $false
            Write-Warning "Rollback left installation files in $stage. $($_.Exception.Message)"
        }
        throw $failure
    } finally {
        if ($cleanupStage) { Remove-PayloadStage $stage }
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
