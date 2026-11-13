# Deploy an existing packaged companion beside a development host; no build.
param(
    [string]$ArtifactDirectory = (Join-Path $PSScriptRoot '../artifacts/joint'),
    [string]$HostExecutable = (Join-Path $PSScriptRoot '../../bgxiong-ai-story/src-tauri/target/debug/bgxiong-ai-story-tauri.exe')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$hostFile = Get-Item -LiteralPath $HostExecutable
if ($hostFile.PSIsContainer -or $hostFile.Name -ne 'bgxiong-ai-story-tauri.exe') {
    throw 'Select an existing development host executable.'
}
$receipt = Get-Content -LiteralPath (Join-Path $ArtifactDirectory 'component.json') -Raw | ConvertFrom-Json
$name = 'bgxiong-any2text.exe'
if ($receipt.schemaVersion -ne 1 -or $receipt.protocolVersion -ne 1 -or $receipt.executable -cne $name) {
    throw 'Unsupported component receipt.'
}
$source = Get-Item -LiteralPath (Join-Path $ArtifactDirectory $name)
$hash = (Get-FileHash -LiteralPath $source.FullName -Algorithm SHA256).Hash
if ($source.Length -ne $receipt.bytes -or $hash -ne $receipt.sha256) {
    throw 'Component size or SHA256 does not match its receipt. Rebuild the component.'
}
$hostRoot = Join-Path (Split-Path -Parent $root) 'bgxiong-ai-story'
foreach ($schema in @((Join-Path $root 'contracts/v1/service-provider.schema.json'),
    (Join-Path $hostRoot 'contracts/any2text/v1/service-provider.schema.json'))) {
    if ((Get-FileHash -LiteralPath $schema -Algorithm SHA256).Hash -ne $receipt.schemaSha256) {
        throw "Protocol schema mismatch: $schema"
    }
}
$destination = Join-Path $hostFile.DirectoryName $name
if (Test-Path -LiteralPath $destination) {
    if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $hash) {
        throw "A different companion already exists. Close it and preserve it before replacing: $destination"
    }
} else {
    Copy-Item -LiteralPath $source.FullName -Destination $destination -ErrorAction Stop
}
if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $hash) {
    throw 'Deployed companion verification failed.'
}
$deployment = [ordered]@{
    deployedAt = [DateTimeOffset]::Now.ToString('o')
    hostExecutable = $hostFile.FullName
    companionExecutable = $destination
    sourceExecutable = $source.FullName
    sha256 = $hash.ToLowerInvariant()
    component = $receipt
}
$deployment | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $hostFile.DirectoryName 'any2text-development.json') -Encoding UTF8
Write-Output "Verified companion: $destination"
Write-Output 'Click the host companion button. Restart the host only if its Bridge was initialized without Any2Text support.'
