#Requires -Version 5.1

[CmdletBinding()]
param([switch] $SkipBuild)

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

$repo = Split-Path -Parent $PSScriptRoot
$documents = @('README.md', 'LICENSE', 'THIRD-PARTY-LICENSES')
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
    foreach ($directory in @($portable, $install, $output)) { $null = [IO.Directory]::CreateDirectory($directory) }
    [IO.File]::Copy($binary, (Join-Path $portable 'BetterExplorer.exe'))
    foreach ($name in $documents) {
        [IO.File]::Copy((Join-Path $repo $name), (Join-Path $portable $name))
    }
    foreach ($name in @('BetterExplorer.exe') + $documents) {
        [IO.File]::Copy((Join-Path $portable $name), (Join-Path $install $name))
    }
    foreach ($packageRoot in @($portable, $install)) {
        $assetDirectory = Join-Path $packageRoot 'assets'
        $null = [IO.Directory]::CreateDirectory($assetDirectory)
        [IO.File]::Copy((Join-Path $repo 'assets/bex.png'), (Join-Path $assetDirectory 'bex.png'))
        Copy-Item -LiteralPath (Join-Path $repo 'docs') -Destination $packageRoot -Recurse
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

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $portableZip = "$prefix-portable.zip"
    $installZip = "$prefix-install.zip"
    [IO.Compression.ZipFile]::CreateFromDirectory($portable, (Join-Path $output $portableZip))
    [IO.Compression.ZipFile]::CreateFromDirectory($install, (Join-Path $output $installZip))
    $sumLines = foreach ($name in @($portableZip, $installZip)) {
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
