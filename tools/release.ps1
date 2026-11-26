#Requires -Version 5.1

[CmdletBinding()]
param([switch] $SkipBuild, [switch] $PortableOnly, [string] $InnoCompiler)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-common.ps1')

function Assert-X64Executable {
    param([string] $Path)

    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Expected a Windows executable at $Path." }
        $reader.BaseStream.Position = 0x3c
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "Expected an x64 Windows executable at $Path."
        }
    } finally {
        $reader.Dispose()
    }
}

function Find-InnoCompiler {
    if ($InnoCompiler) {
        return (Get-Item -LiteralPath $InnoCompiler -ErrorAction Stop).FullName
    }
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    foreach ($parent in @("$env:LOCALAPPDATA\Programs", ${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        foreach ($folder in 'Inno Setup 7', 'Inno Setup 6') {
            if (-not $parent) { continue }
            $candidate = Join-Path $parent "$folder\ISCC.exe"
            if ([IO.File]::Exists($candidate)) { return $candidate }
        }
    }
    throw 'Install Inno Setup 6.6 or newer, or pass -InnoCompiler with the path to ISCC.exe.'
}

$repo = Split-Path -Parent $PSScriptRoot
$documents = @('README.md', 'LICENSE', 'THIRD-PARTY-LICENSES', 'CHANGELOG.md', 'CONTRIBUTING.md', 'SECURITY.md')
foreach ($name in $documents) { $null = Get-OwnedFileHash (Join-Path $repo $name) }

Push-Location -LiteralPath $repo
try {
    $metadataJson = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo package metadata.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object Name -ceq 'better-explorer')
    if ($package.Count -ne 1) { throw 'Expected one better-explorer package.' }
    if (-not $SkipBuild) {
        & cargo build --locked --release --bin BetterExplorer
        if ($LASTEXITCODE -ne 0) { throw 'Cargo release build failed.' }
    }
} finally {
    Pop-Location
}

$version = $package[0].version
$binary = Join-Path $metadata.target_directory 'release/BetterExplorer.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Missing release binary $binary." }
Assert-X64Executable $binary

$dist = Join-Path $repo 'dist'
if (-not (Test-Path -LiteralPath $dist)) { $null = New-Item -ItemType Directory -Path $dist }
Assert-PlainDirectory $dist
$dist = (Get-Item -LiteralPath $dist).FullName
$id = [guid]::NewGuid().ToString('N')
$timestamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmssfff', [Globalization.CultureInfo]::InvariantCulture)
$prefix = "BetterExplorer-$version-windows-x64"
$workName = ".build-$id"
$work = Join-Path $dist $workName
$portable = Join-Path $work 'portable'
$install = Join-Path $work 'install'
$output = Join-Path $work 'output'
$destination = Join-Path $dist "$prefix-$timestamp"

$null = New-Item -ItemType Directory -Path $work
try {
    $null = [IO.Directory]::CreateDirectory($output)
    $null = [IO.Directory]::CreateDirectory($portable)
    [IO.File]::Copy($binary, (Join-Path $portable 'BetterExplorer.exe'))
    foreach ($name in $documents) {
        [IO.File]::Copy((Join-Path $repo $name), (Join-Path $portable $name))
    }
    $assetDirectory = Join-Path $portable 'assets'
    $null = [IO.Directory]::CreateDirectory($assetDirectory)
    [IO.File]::Copy((Join-Path $repo 'assets/bex.png'), (Join-Path $assetDirectory 'bex.png'))
    $screenshots = Join-Path $repo 'assets/screenshots'
    if (Test-Path -LiteralPath $screenshots -PathType Container) {
        Copy-Item -LiteralPath $screenshots -Destination $assetDirectory -Recurse
    }
    Copy-Item -LiteralPath (Join-Path $repo 'docs') -Destination $portable -Recurse

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $portableZip = "$prefix-portable.zip"
    [IO.Compression.ZipFile]::CreateFromDirectory($portable, (Join-Path $output $portableZip))
    $archives = @($portableZip)
    if (-not $PortableOnly) {
        $null = [IO.Directory]::CreateDirectory($install)
        foreach ($name in @('BetterExplorer.exe', 'README.md', 'LICENSE', 'THIRD-PARTY-LICENSES')) {
            [IO.File]::Copy((Join-Path $portable $name), (Join-Path $install $name))
        }
        foreach ($name in @('install.ps1', 'uninstall.ps1', 'install-common.ps1')) {
            [IO.File]::Copy((Join-Path $PSScriptRoot $name), (Join-Path $install $name))
        }

        $checksums = [ordered] @{}
        foreach ($name in $script:BetterExplorerPayload) {
            $checksums[$name] = Get-OwnedFileHash (Join-Path $install $name)
        }
        $manifest = [ordered] @{
            product = 'BetterExplorer'
            schema = 1
            version = $version
            files = $checksums
            shortcut_sha256 = $null
        }
        Write-PayloadManifest $manifest (Join-Path $install 'bundle-manifest.json')
        $setupBaseName = "$prefix-setup"
        $compiler = Find-InnoCompiler
        # The compiler transcript goes to the host and leaves the destination as the only output value.
        $compilerArguments = @(
            '/Qp', "/DAppVersion=$version", "/DPayloadDir=$install",
            "/DOutputBaseName=$setupBaseName", "/O$output", (Join-Path $PSScriptRoot 'setup.iss')
        )
        & $compiler @compilerArguments | Out-Host
        $setup = "$setupBaseName.exe"
        if ($LASTEXITCODE -ne 0 -or -not [IO.File]::Exists((Join-Path $output $setup))) {
            throw 'BetterExplorer Setup compilation failed.'
        }
        $archives += $setup
    }
    $sumLines = foreach ($name in $archives) {
        $hash = Get-OwnedFileHash (Join-Path $output $name)
        "$hash *$name"
    }
    [IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS.txt'), [string[]] $sumLines)
    [IO.Directory]::Move($output, $destination)
    Write-Host "Release archives and checksums are in $destination."
    Write-Output $destination
} finally {
    $staging = Get-Item -LiteralPath $work -Force
    if ($staging.Name -cne $workName -or $staging.Parent.FullName -ne $dist -or
        ($staging.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected release staging under $dist. The directory at $work was kept."
    }
    Remove-Item -LiteralPath $staging.FullName -Recurse -Force
}
