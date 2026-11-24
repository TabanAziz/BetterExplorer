$script:BetterExplorerPayload = @(
    'BetterExplorer.exe', 'README.md', 'LICENSE', 'THIRD-PARTY-LICENSES', 'uninstall.ps1', 'install-common.ps1'
)
$script:BetterExplorerInstallManifest = '.betterexplorer-install.json'

function Get-OwnedFileHash {
    param([string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a regular file at $Path."
    }
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
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
    param([scriptblock] $Action)

    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $mutex = [Threading.Mutex]::new($false, "Global\BetterExplorer.Install.$sid")
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
