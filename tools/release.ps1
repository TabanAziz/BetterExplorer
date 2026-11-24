#Requires -Version 5.1

[CmdletBinding()]
param([switch] $SkipBuild, [switch] $PortableOnly)

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

function Build-SetupLauncher {
    param([string] $Repo, [string] $Work, [string] $Destination)

    $compiler = Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    $automation = Join-Path $env:SystemRoot 'Microsoft.NET\assembly\GAC_MSIL\System.Management.Automation\v4.0_3.0.0.0__31bf3856ad364e35\System.Management.Automation.dll'
    foreach ($dependency in @($compiler, $automation)) {
        if (-not [IO.File]::Exists($dependency)) { throw "Required Windows setup build dependency is missing: $dependency" }
    }

    $common = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'install-common.ps1'))
    $import = '. (Join-Path $PSScriptRoot ''install-common.ps1'')'
    $resources = foreach ($scriptName in @('Install', 'Uninstall')) {
        $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot ($scriptName.ToLowerInvariant() + '.ps1')))
        if ([regex]::Matches($source, [regex]::Escape($import)).Count -ne 1) {
            throw "Expected one common-script import in $scriptName.ps1."
        }
        $embedded = $source.Replace($import, $common).Replace('$PSScriptRoot', '$BetterExplorerBundleRoot')
        $tokens = $null
        $parseErrors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseInput($embedded, [ref] $tokens, [ref] $parseErrors)
        if ($parseErrors.Count) {
            throw "The embedded $scriptName script has syntax errors: $($parseErrors.Message -join ' ')"
        }
        $resourcePath = Join-Path $Work "$scriptName.embedded.ps1"
        [IO.File]::WriteAllText($resourcePath, $embedded)
        "/resource:$resourcePath,BetterExplorer.$scriptName.ps1"
    }

    $setup = Join-Path $Destination 'Setup.exe'
    $icon = Join-Path $Repo 'assets/bex.ico'
    $source = Join-Path $PSScriptRoot 'SetupLauncher.cs'
    $compilerOptions = @(
        '/nologo', '/target:winexe', '/platform:x64', '/optimize+',
        "/out:$setup", "/win32icon:$icon", "/reference:$automation", '/reference:System.Windows.Forms.dll'
    )
    & $compiler @compilerOptions @resources $source
    if ($LASTEXITCODE -ne 0) { throw 'Setup.exe compilation failed.' }
    Assert-X64Executable $setup
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
    $packageRoots = @($portable)
    if (-not $PortableOnly) { $packageRoots += $install }
    $null = [IO.Directory]::CreateDirectory($output)
    foreach ($packageRoot in $packageRoots) {
        $null = [IO.Directory]::CreateDirectory($packageRoot)
        [IO.File]::Copy($binary, (Join-Path $packageRoot 'BetterExplorer.exe'))
        foreach ($name in $documents) {
            [IO.File]::Copy((Join-Path $repo $name), (Join-Path $packageRoot $name))
        }
        $assetDirectory = Join-Path $packageRoot 'assets'
        $null = [IO.Directory]::CreateDirectory($assetDirectory)
        [IO.File]::Copy((Join-Path $repo 'assets/bex.png'), (Join-Path $assetDirectory 'bex.png'))
        $screenshots = Join-Path $repo 'assets/screenshots'
        if (Test-Path -LiteralPath $screenshots -PathType Container) {
            Copy-Item -LiteralPath $screenshots -Destination $assetDirectory -Recurse
        }
        Copy-Item -LiteralPath (Join-Path $repo 'docs') -Destination $packageRoot -Recurse
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $portableZip = "$prefix-portable.zip"
    [IO.Compression.ZipFile]::CreateFromDirectory($portable, (Join-Path $output $portableZip))
    $archives = @($portableZip)
    if (-not $PortableOnly) {
        foreach ($name in @('install.ps1', 'uninstall.ps1', 'install-common.ps1')) {
            [IO.File]::Copy((Join-Path $PSScriptRoot $name), (Join-Path $install $name))
        }
        Build-SetupLauncher -Repo $repo -Work $work -Destination $install

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
        $installZip = "$prefix-install.zip"
        [IO.Compression.ZipFile]::CreateFromDirectory($install, (Join-Path $output $installZip))
        $archives += $installZip
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
