param(
    [switch]$SkipBuild,
    [string]$OutputDirectory
)
$ErrorActionPreference = "Stop"
$workspace = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $workspace "target/integrations" }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $workspace $OutputDirectory }
$manifest = Get-Content (Join-Path $workspace "Cargo.toml") -Raw -Encoding UTF8
$version = [regex]::Match($manifest, '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (-not $version) { throw "Cannot determine workspace version." }
Push-Location $workspace
try {
    if (-not $SkipBuild) {
        cargo build --release -p unzip-mcp --locked
        if ($LASTEXITCODE -ne 0) { throw "MCP release build failed." }
    }
    $binary = Join-Path $workspace "target/release/unzip-mcp.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Build unzip-mcp.exe before packaging." }
    $builtVersion = (& $binary --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $builtVersion -ne "unzip-mcp $version") { throw "Binary version mismatch: $builtVersion, expected $version." }
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $stage = Join-Path $OutputDirectory ("stage-" + [guid]::NewGuid().ToString("N"))
    $package = Join-Path $stage "unzip-rs-mcp"
    New-Item -ItemType Directory -Force -Path $package | Out-Null
    Copy-Item -LiteralPath $binary -Destination $package
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "README.md") -Destination $package
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "examples") -Destination $package -Recurse
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "skills") -Destination $package -Recurse
    $toolsDirectory = Join-Path $package "resources/bundled"
    New-Item -ItemType Directory -Force -Path $toolsDirectory | Out-Null
    Copy-Item -Path (Join-Path $workspace "assets/bundled/*") -Destination $toolsDirectory -Recurse
    $license = Join-Path $workspace "LICENSE"
    if (Test-Path -LiteralPath $license) { Copy-Item -LiteralPath $license -Destination (Join-Path $package "LICENSE-unzip-rs") }
    $archive = Join-Path $OutputDirectory "unzip-rs-mcp_$($version)_windows-x86_64.zip"
    Compress-Archive -LiteralPath $package -DestinationPath $archive -Force
    # Remove only this invocation's generated staging directory.
    $resolvedOutput = (Resolve-Path -LiteralPath $OutputDirectory).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
    $resolvedStage = (Resolve-Path -LiteralPath $stage).Path
    if (-not $resolvedStage.StartsWith($resolvedOutput + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Generated staging directory escaped the output directory."
    }
    Remove-Item -LiteralPath $stage -Recurse -Force
    Write-Output "MCP integration bundle: $archive"
} finally { Pop-Location }
