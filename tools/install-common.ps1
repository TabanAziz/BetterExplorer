$script:BetterExplorerPayload = @(
    'BetterExplorer.exe', 'README.md', 'LICENSE', 'THIRD-PARTY-LICENSES', 'uninstall.ps1', 'install-common.ps1'
)
$script:BetterExplorerInstallManifest = '.betterexplorer-install.json'
$script:BetterExplorerWinERunKey = 'Software\Microsoft\Windows\CurrentVersion\Run'
$script:BetterExplorerWinERunName = 'BetterExplorerWinE'

function Get-OwnedFileHash {
    param([string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a regular file at $Path."
    }
    $stream = [IO.File]::OpenRead($item.FullName)
    try {
        $hash = [Security.Cryptography.SHA256]::Create()
        try {
            [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
        } finally {
            $hash.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function Assert-PlainDirectory {
    param([string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a regular directory at $Path."
    }
}

function Get-InstallLocations {
    param([string] $TestRoot)

    if ($TestRoot) {
        $temp = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).ProviderPath.TrimEnd('\')
        $programs = (Resolve-Path -LiteralPath $TestRoot).ProviderPath
        $outsideTemp = [string]::IsNullOrEmpty([IO.Path]::GetDirectoryName($programs)) -or
            [IO.Path]::GetDirectoryName($programs).TrimEnd('\') -ne $temp
        if ($outsideTemp) { throw 'TestRoot must be an existing direct child of the Windows temp directory.' }
        Assert-PlainDirectory $programs
        $shortcut = Join-Path $programs 'BetterExplorer.lnk'
    } else {
        $local = [Environment]::GetFolderPath('LocalApplicationData')
        $startMenu = [Environment]::GetFolderPath('Programs', 'DoNotVerify')
        if (-not $local -or -not $startMenu) { throw 'Windows did not return the current user program folders.' }
        $programs = Join-Path $local 'Programs'
        $shortcut = Join-Path $startMenu 'BetterExplorer.lnk'
    }

    [pscustomobject] @{
        Programs = $programs
        Application = Join-Path $programs 'BetterExplorer'
        Shortcut = $shortcut
    }
}

function Invoke-InstallLock {
    param([scriptblock] $Action, [string] $TestRoot)

    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $name = "Global\BetterExplorer.Install.$sid"
    if ($TestRoot) {
        $name = "Local\BetterExplorer.TestInstall.$sid.$([IO.Path]::GetFileName($TestRoot).ToUpperInvariant())"
    }
    $mutex = [Threading.Mutex]::new($false, $name)
    $acquired = $false
    try {
        try { $acquired = $mutex.WaitOne(10000) } catch [Threading.AbandonedMutexException] { $acquired = $true }
        if (-not $acquired) { throw 'Another BetterExplorer installation operation is running.' }
        & $Action
    } finally {
        if ($acquired) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
}

function Read-PayloadManifest {
    param([string] $Directory, [string] $Name, [switch] $AllowMissingFiles)

    Assert-PlainDirectory $Directory
    $path = Join-Path $Directory $Name
    $null = Get-OwnedFileHash $path
    $manifest = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if ($manifest.product -cne 'BetterExplorer' -or $manifest.schema -ne 1 -or
        $manifest.version -cnotmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]+)?$' -or
        ($null -ne $manifest.shortcut_sha256 -and $manifest.shortcut_sha256 -cnotmatch '^[a-f0-9]{64}$') -or
        @($manifest.files.PSObject.Properties).Count -ne $script:BetterExplorerPayload.Count) {
        throw "Invalid BetterExplorer file manifest at $path."
    }
    foreach ($fileName in $script:BetterExplorerPayload) {
        $record = $manifest.files.PSObject.Properties[$fileName]
        if ($null -eq $record -or $record.Value -cnotmatch '^[a-f0-9]{64}$') {
            throw "Invalid checksum for $fileName in $path."
        }
        $file = Join-Path $Directory $fileName
        if ($AllowMissingFiles -and -not (Test-Path -LiteralPath $file)) { continue }
        if ((Get-OwnedFileHash $file) -cne $record.Value) {
            throw "Checksum mismatch for $file. The file may have changed since it was packaged or installed."
        }
    }
    $manifest
}

function Write-PayloadManifest {
    param([object] $Manifest, [string] $Path)

    $json = $Manifest | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine)
}

function Get-InstalledShortcutHash {
    param([string] $Path, [string] $Expected)

    if (-not $Path -or -not $Expected -or -not (Test-Path -LiteralPath $Path)) { return $null }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { return $null }
    $actual = Get-OwnedFileHash $Path
    if ($actual -ceq $Expected) { return $actual }
    return $null
}

function Test-OwnedWinERunValue {
    param([Microsoft.Win32.RegistryKey] $Key, [string] $Executable)

    if ($null -eq $Key) { return $false }
    $name = $script:BetterExplorerWinERunName
    $value = $Key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $expected = '"{0}" --win-e-listener' -f $Executable
    $value -is [string] -and [string]::Equals($value, $expected, [StringComparison]::Ordinal) -and
        $Key.GetValueKind($name) -eq [Microsoft.Win32.RegistryValueKind]::String
}

function Test-OwnedWinERegistration {
    param([string] $Executable)

    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($script:BetterExplorerWinERunKey)
    try {
        Test-OwnedWinERunValue $key $Executable
    } finally {
        if ($null -ne $key) { $key.Dispose() }
    }
}

function Test-OtherWinERegistration {
    param([string] $Executable)

    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($script:BetterExplorerWinERunKey)
    if ($null -eq $key) { return $false }
    try {
        ($key.GetValueNames() -icontains $script:BetterExplorerWinERunName) -and
            -not (Test-OwnedWinERunValue $key $Executable)
    } finally {
        $key.Dispose()
    }
}

function Set-LegacyWinERegistration {
    param([string] $Executable, [bool] $Enabled)

    $user = [Microsoft.Win32.Registry]::CurrentUser
    $key = if ($Enabled) { $user.CreateSubKey($script:BetterExplorerWinERunKey) }
        else { $user.OpenSubKey($script:BetterExplorerWinERunKey, $true) }
    if ($null -eq $key) { return }
    try {
        $name = $script:BetterExplorerWinERunName
        $owned = Test-OwnedWinERunValue $key $Executable
        if ($Enabled -and -not $owned) {
            if ($key.GetValueNames() -icontains $name) {
                throw 'The Win+E startup entry belongs to a different program.'
            }
            $key.SetValue($name, ('"{0}" --win-e-listener' -f $Executable), [Microsoft.Win32.RegistryValueKind]::String)
        } elseif (-not $Enabled -and $owned) {
            $key.DeleteValue($name, $false)
        }
    } finally {
        $key.Dispose()
    }
}

function Invoke-WinEControl {
    param([string] $Executable, [string] $Command)

    $process = Start-Process -FilePath $Executable -ArgumentList $Command -PassThru -ErrorAction Stop
    try {
        # The control process can start a listener that stays running after setup exits.
        if (-not $process.WaitForExit(30000)) {
            try {
                $process.Kill()
                $null = $process.WaitForExit(1000)
            } catch [InvalidOperationException] {}
            throw "The BetterExplorer Win+E command $Command timed out."
        }
        $code = $process.ExitCode
        if ($Command -eq '--win-e-enabled' -and $code -eq 2) { return $false }
        if ($code -ne 0) { throw "The BetterExplorer Win+E command $Command returned exit code $code." }
        return $true
    } finally {
        $process.Dispose()
    }
}

function Get-InstalledWinEState {
    param([string] $Executable, [object] $Manifest, [string] $TestRoot)

    $canControl = -not $TestRoot -and $Manifest -and
        $Manifest.PSObject.Properties['win_e_cli'] -and $Manifest.win_e_cli -eq 1 -and
        [IO.File]::Exists($Executable)
    $enabled = if ($canControl) { Invoke-WinEControl $Executable '--win-e-enabled' }
        else { -not $TestRoot -and (Test-OwnedWinERegistration $Executable) }
    [pscustomobject] @{ Enabled = $enabled; CanControl = $canControl }
}

function Restore-InstalledWinE {
    param([string] $Executable, [bool] $CanControl)

    if ($CanControl) {
        $configured = Invoke-WinEControl $Executable '--win-e-enabled'
        $command = if ($configured) { '--win-e-resume' } else { '--win-e-on' }
        $null = Invoke-WinEControl $Executable $command
    } else {
        Set-LegacyWinERegistration $Executable $true
    }
}

function Remove-PayloadStage {
    param([string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) { return }
    try {
        Assert-PlainDirectory $Path
        foreach ($name in $script:BetterExplorerPayload + $script:BetterExplorerInstallManifest) {
            $file = Join-Path $Path $name
            if (Test-Path -LiteralPath $file -PathType Leaf) { [IO.File]::Delete($file) }
        }
        [IO.Directory]::Delete($Path, $false)
    } catch {
        Write-Warning "Could not remove the temporary installation directory $Path. $($_.Exception.Message)"
    }
}
