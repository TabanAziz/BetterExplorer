#Requires -Version 5.1

[CmdletBinding()]
param(
    [switch] $StartMenuShortcut,
    [string] $TestRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-common.ps1')

$paths = Get-InstallLocations $TestRoot
$bundle = Read-PayloadManifest -Directory $PSScriptRoot -Name 'bundle-manifest.json'

Invoke-InstallLock {
    if (-not (Test-Path -LiteralPath $paths.Programs)) {
        $null = [IO.Directory]::CreateDirectory($paths.Programs)
    }
    Assert-PlainDirectory $paths.Programs

    $previous = $null
    $shortcutHash = $null
    if (Test-Path -LiteralPath $paths.Application) {
        $previous = Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest
        $managed = $script:BetterExplorerPayload + $script:BetterExplorerInstallManifest
        $unmanaged = @(Get-ChildItem -LiteralPath $paths.Application -Force | Where-Object Name -notin $managed)
        if ($unmanaged.Count -gt 0) { throw "The installation folder contains additional files. Keep them elsewhere before updating $($paths.Application)." }
        $shortcutHash = Get-InstalledShortcutHash $paths.Shortcut $previous.shortcut_sha256
    }

    $createShortcut = $StartMenuShortcut -and -not $shortcutHash
    if ($createShortcut -and (Test-Path -LiteralPath $paths.Shortcut)) {
        throw "A separate item already occupies the Start Menu shortcut path $($paths.Shortcut)."
    }

    $id = [guid]::NewGuid().ToString('N')
    $stage = Join-Path $paths.Programs ".BetterExplorer-new-$id"
    $backup = Join-Path $paths.Programs ".BetterExplorer-old-$id"
    $shortcutStage = $null
    $oldMoved = $false
    $newMoved = $false
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
            Assert-PlainDirectory $shortcutDirectory
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
        }
        Write-PayloadManifest $manifest (Join-Path $stage $script:BetterExplorerInstallManifest)

        if ($previous) {
            [IO.Directory]::Move($paths.Application, $backup)
            $oldMoved = $true
        }
        [IO.Directory]::Move($stage, $paths.Application)
        $newMoved = $true
        if ($createShortcut) { [IO.File]::Move($shortcutStage, $paths.Shortcut) }
        $completed = $true
    } catch {
        $failure = $_
        try {
            if ($newMoved) { [IO.Directory]::Move($paths.Application, $stage) }
            if ($oldMoved) { [IO.Directory]::Move($backup, $paths.Application) }
        } catch {
            $cleanupStage = $false
            Write-Warning "Rollback left files in $stage or $backup. $($_.Exception.Message)"
        }
        throw $failure
    } finally {
        if ($cleanupStage) { Remove-PayloadStage $stage }
        if ($completed -and $oldMoved) { Remove-PayloadStage $backup }
        if ($shortcutStage -and [IO.File]::Exists($shortcutStage)) { [IO.File]::Delete($shortcutStage) }
    }

    Write-Host "Installed BetterExplorer $($bundle.version) in $($paths.Application)."
}
