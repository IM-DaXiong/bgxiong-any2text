param([string] $OutputDirectory = 'L:/00Dev/bgxiong-00-release/dev-bgxiong-any2text')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$output = (New-Item -ItemType Directory -Force -Path $OutputDirectory).FullName
$source = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify the Any2Text source revision.' }
Push-Location $root
try {
    $hostRoot = Join-Path (Split-Path -Parent $root) 'bgxiong-ai-story'
    if (Test-Path -LiteralPath (Join-Path $hostRoot 'ui/src/theme/themeRegistry.ts')) {
        & node scripts/sync-bigbear-theme.mjs --check
        if ($LASTEXITCODE -ne 0) { throw 'Any2Text theme assets differ from the BigBear source.' }
    }
    & npm.cmd ci --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'Any2Text dependency installation failed.' }
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Any2Text frontend build failed.' }
    # Compiler cache stays internal; never mix it with deliverables.
    $target = Join-Path $root 'artifacts/joint/cargo-target'
    & cargo build --manifest-path src-tauri/Cargo.toml --target-dir $target --release --locked --features custom-protocol
    if ($LASTEXITCODE -ne 0) { throw 'Any2Text release executable build failed.' }
    if ((& git rev-parse HEAD).Trim() -ne $source) { throw 'The Any2Text source revision changed during the build.' }
    $exe = Join-Path $output 'bgxiong-any2text.exe'
    Copy-Item -LiteralPath (Join-Path $target 'release/bgxiong-any2text.exe') -Destination $exe
    $schema = Join-Path $root 'contracts/v1/service-provider.schema.json'
    $receipt = [ordered]@{
        schemaVersion = 1
        sourceCommit = $source
        sourceDirty = [bool](& git status --porcelain --untracked-files=normal)
        protocolVersion = 1
        schemaSha256 = (Get-FileHash -LiteralPath $schema -Algorithm SHA256).Hash.ToLowerInvariant()
        executable = 'bgxiong-any2text.exe'
        bytes = (Get-Item -LiteralPath $exe).Length
        sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $receipt | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'component.json') -Encoding UTF8
    $receipt | ConvertTo-Json
} finally { Pop-Location }
