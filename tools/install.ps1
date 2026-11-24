#Requires -Version 5.1

[CmdletBinding()]
param(
    [switch] $StartMenuShortcut,
    [switch] $Quiet,
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

function Show-SetupDialog {
    param([bool] $WinEEnabled, [int] $FolderManagerState, [string] $Version, [string] $Executable)

    $form = [Windows.Forms.Form]::new()
    $icon = $null
    $image = $null
    try {
        $form.Text = 'BetterExplorer Setup'
        $form.ClientSize = [Drawing.Size]::new(460, 343)
        $form.AutoScaleDimensions = [Drawing.SizeF]::new(96, 96)
        $form.AutoScaleMode = [Windows.Forms.AutoScaleMode]::Dpi
        $form.StartPosition = [Windows.Forms.FormStartPosition]::CenterScreen
        $form.FormBorderStyle = [Windows.Forms.FormBorderStyle]::FixedDialog
        $form.MinimizeBox = $false
        $form.MaximizeBox = $false
        $form.Font = [Drawing.Font]::new('Segoe UI', 10)
        $form.BackColor = [Drawing.SystemColors]::Window

        $header = [Windows.Forms.Panel]::new()
        $header.Dock = [Windows.Forms.DockStyle]::Top
        $header.Height = 78
        $header.BackColor = [Drawing.Color]::FromArgb(21, 46, 83)
        $icon = [Drawing.Icon]::ExtractAssociatedIcon($Executable)
        if ($null -ne $icon) {
            $form.Icon = $icon
            $image = $icon.ToBitmap()
            $picture = [Windows.Forms.PictureBox]::new()
            $picture.SetBounds(23, 17, 44, 44)
            $picture.Image = $image
            $picture.SizeMode = [Windows.Forms.PictureBoxSizeMode]::Zoom
            $header.Controls.Add($picture)
        }

        $title = [Windows.Forms.Label]::new()
        $title.AutoSize = $true
        $title.Location = [Drawing.Point]::new(80, 21)
        $title.Text = "BetterExplorer $Version"
        $title.Font = [Drawing.Font]::new('Segoe UI', 18, [Drawing.FontStyle]::Bold)
        $title.ForeColor = [Drawing.Color]::White
        $header.Controls.Add($title)

        $question = [Windows.Forms.Label]::new()
        $question.Text = 'What should open when you press Win+E?'
        $question.AutoSize = $true
        $question.Location = [Drawing.Point]::new(24, 101)

        $windowsOption = [Windows.Forms.RadioButton]::new()
        $windowsOption.Text = 'Windows File Explorer'
        $windowsOption.AutoSize = $true
        $windowsOption.Location = [Drawing.Point]::new(24, 131)
        $windowsOption.Checked = -not $WinEEnabled
        $windowsOption.TabIndex = 0

        $betterOption = [Windows.Forms.RadioButton]::new()
        $betterOption.Text = 'BetterExplorer'
        $betterOption.AutoSize = $true
        $betterOption.Location = [Drawing.Point]::new(24, 158)
        $betterOption.Checked = $WinEEnabled
        $betterOption.TabIndex = 1

        $folderOption = [Windows.Forms.CheckBox]::new()
        $folderOption.Text = 'Use BetterExplorer as the default folder manager'
        $folderOption.AutoSize = $true
        $folderOption.Location = [Drawing.Point]::new(24, 198)
        $initialFolderChoice = switch ($FolderManagerState) {
            0 { [Windows.Forms.CheckState]::Checked }
            3 { [Windows.Forms.CheckState]::Indeterminate }
            default { [Windows.Forms.CheckState]::Unchecked }
        }
        $folderOption.CheckState = $initialFolderChoice
        $folderOption.Enabled = $FolderManagerState -ne 4
        $folderOption.TabIndex = 2

        $folderHint = [Windows.Forms.Label]::new()
        $folderHint.Text = switch ($FolderManagerState) {
            3 { 'The folder default changed. Select or clear to update it.' }
            4 { 'Another registration uses the BetterExplorer folder command.' }
            default { 'For folders and drives opened through the Windows shell.' }
        }
        $folderHint.AutoSize = $true
        $folderHint.ForeColor = [Drawing.SystemColors]::GrayText
        $folderHint.Location = [Drawing.Point]::new(43, 224)

        $shortcutOption = [Windows.Forms.CheckBox]::new()
        $shortcutOption.Text = 'Create a Start menu shortcut'
        $shortcutOption.AutoSize = $true
        $shortcutOption.Location = [Drawing.Point]::new(24, 256)
        $shortcutOption.TabIndex = 3

        $installButton = [Windows.Forms.Button]::new()
        $installButton.Text = 'Install'
        $installButton.SetBounds(246, 296, 90, 34)
        $installButton.TabIndex = 4
        $installButton.DialogResult = [Windows.Forms.DialogResult]::OK

        $cancelButton = [Windows.Forms.Button]::new()
        $cancelButton.Text = 'Cancel'
        $cancelButton.SetBounds(346, 296, 90, 34)
        $cancelButton.TabIndex = 5
        $cancelButton.DialogResult = [Windows.Forms.DialogResult]::Cancel

        $form.Controls.AddRange(@($header, $question, $windowsOption, $betterOption, $folderOption, $folderHint, $shortcutOption, $installButton, $cancelButton))
        $form.AcceptButton = $installButton
        $form.CancelButton = $cancelButton
        if ($form.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { return }
        $folderManagerChoice = if ($folderOption.Enabled -and $folderOption.CheckState -ne $initialFolderChoice) {
            if ($folderOption.Checked) { 'BetterExplorer' } else { 'WindowsExplorer' }
        }
        [pscustomobject] @{
            WinE = if ($betterOption.Checked) { 'BetterExplorer' } else { 'WindowsExplorer' }
            FolderManager = $folderManagerChoice
            StartMenuShortcut = $shortcutOption.Checked
        }
    } finally {
        $form.Dispose()
        if ($null -ne $image) { $image.Dispose() }
        if ($null -ne $icon) { $icon.Dispose() }
    }
}

$script:winEConflictNotice = $null
$install = {
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

    $executable = Join-Path $paths.Application 'BetterExplorer.exe'
    $previousWinE = Get-InstalledWinEState $executable $previous $TestRoot
    $enableWinE = $WinE -eq 'BetterExplorer' -or (-not $WinE -and $previous -and $previousWinE.Enabled)
    if (-not $TestRoot -and $WinE -and (Test-OtherWinERegistration $executable)) {
        $message = 'Win+E is managed by another copy of BetterExplorer. Turn it off in that copy to change it here.'
        if ($enableWinE) { throw $message }
        $script:winEConflictNotice = $message
        Write-Warning $message
    }
    $createShortcut = $addShortcut -and -not $shortcutHash
    if ($createShortcut -and (Test-Path -LiteralPath $paths.Shortcut)) {
        throw "A separate item already occupies the Start Menu shortcut path $($paths.Shortcut)."
    }

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
                else { '--win-e-on' }
            $winEChangeAttempted = $true
            $null = Invoke-WinEControl $executable $command
        }
        if ($createShortcut) {
            [IO.File]::Move($shortcutStage, $paths.Shortcut)
            $shortcutMoved = $true
        }
        if ($FolderManager) {
            $command = if ($FolderManager -eq 'BetterExplorer') { '--folder-manager-on' }
                else { '--folder-manager-off' }
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

    Write-Host "Installed BetterExplorer $($bundle.version) in $($paths.Application)."
}

$interactive = -not $Quiet -and $PSBoundParameters.Count -eq 0
$hostedSetup = (Get-Variable BetterExplorerHostedSetup -ValueOnly -ErrorAction SilentlyContinue) -eq $true
if ($interactive) {
    Add-Type -AssemblyName System.Windows.Forms
    [Windows.Forms.Application]::EnableVisualStyles()
}
try {
    $paths = Get-InstallLocations $TestRoot
    $bundle = Read-PayloadManifest -Directory $PSScriptRoot -Name 'bundle-manifest.json'
    $addShortcut = $StartMenuShortcut.IsPresent
    if ($interactive) {
        $installedExecutable = Join-Path $paths.Application 'BetterExplorer.exe'
        $bundleExecutable = Join-Path $PSScriptRoot 'BetterExplorer.exe'
        $currentFolderManager = Invoke-FolderManagerControl $bundleExecutable $installedExecutable '--folder-manager-state'
        $currentWinE = $false
        if (Test-Path -LiteralPath $paths.Application) {
            $installed = Read-PayloadManifest -Directory $paths.Application -Name $script:BetterExplorerInstallManifest
            $currentWinE = (Get-InstalledWinEState $installedExecutable $installed).Enabled
        }
        $options = Show-SetupDialog $currentWinE $currentFolderManager $bundle.version $bundleExecutable
        if (-not $options) { return }
        $WinE = $options.WinE
        if ($options.FolderManager) { $FolderManager = $options.FolderManager }
        $addShortcut = $options.StartMenuShortcut
    }
    $lockRoot = if ($TestRoot) { $paths.Programs }
    Invoke-InstallLock $install -TestRoot $lockRoot
    if ($interactive) {
        $message = "BetterExplorer $($bundle.version) was installed for your account."
        if ($script:winEConflictNotice) { $message += [Environment]::NewLine * 2 + $script:winEConflictNotice }
        $null = [Windows.Forms.MessageBox]::Show($message, 'BetterExplorer Setup', 'OK', 'Information')
    }
} catch {
    if ($interactive -and -not $hostedSetup) {
        $null = [Windows.Forms.MessageBox]::Show(
            $_.Exception.Message, 'BetterExplorer Setup', 'OK', 'Error')
    }
    throw
}
