# T00: call the running BigBear Bridge with a known local ASR configuration.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $MediaPath,
    [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string] $ModelConfigId,
    [string] $Language = 'zh',
    [string] $OutputRoot = (Join-Path $PSScriptRoot '../artifacts/t00'),
    [string] $RegistryPath = (Join-Path $env:LOCALAPPDATA 'BigBear/local-bridge/registry.json'),
    [ValidateRange(1, 3600)] [int] $TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$media = Get-Item -LiteralPath $MediaPath
if ($media.PSIsContainer -or $media.Length -eq 0) { throw 'Expected a non-empty media file.' }

$registry = Get-Content -Raw -LiteralPath $registryPath | ConvertFrom-Json
$base = [uri] $registry.baseUrl
if ($base.Scheme -ne 'http' -or $base.Host -ne '127.0.0.1' -or
    $base.AbsolutePath -ne '/v1/' -or $base.UserInfo -or $base.Query -or $base.Fragment) {
    throw 'Unexpected BigBear Bridge registry address.'
}
$health = Invoke-RestMethod -Uri ([uri]::new($base, 'health')) -TimeoutSec 5 -MaximumRedirection 0
if ($health.code -ne 0 -or $health.data.status -ne 'ok' -or
    $health.data.instanceId -ne $registry.instanceId -or
    'subtitleRest' -notin $health.data.transportFeatures -or
    'asrSelectionV1' -notin $health.data.transportFeatures -or
    $health.data.mediaRuntime.ready -ne $true) {
    throw 'The registered Bridge instance or its media capabilities are not ready.'
}

$runId = 'any2text-t00-' + [guid]::NewGuid().ToString('N')
$run = New-Item -ItemType Directory -Path (Join-Path $OutputRoot $runId)
$inputPath = Join-Path $run.FullName ($runId + $media.Extension)
Copy-Item -LiteralPath $media.FullName -Destination $inputPath
$payload = @{
    filePaths = @($inputPath)
    selection = @{ kind = 'explicit'; modelConfigId = $ModelConfigId }
    outputRequirements = @{ timing = 'segmentTimed'; language = $Language }
    outputMode = 'srtOnly'
} | ConvertTo-Json -Depth 5
$payload | Set-Content -LiteralPath (Join-Path $run.FullName 'request.json') -Encoding UTF8
$health | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run.FullName 'health.json') -Encoding UTF8
Write-Host "Bridge $($registry.instanceId) at $base. Run directory: $($run.FullName)"

$request = @{
    Uri = [uri]::new($base, 'subtitle/recognize')
    Method = 'Post'
    ContentType = 'application/json; charset=utf-8'
    Body = $payload
    UseBasicParsing = $true
    TimeoutSec = $TimeoutSeconds
    MaximumRedirection = 0
}
$timer = [Diagnostics.Stopwatch]::StartNew()
$response = Invoke-WebRequest @request
$timer.Stop()
$response.Content | Set-Content -LiteralPath (Join-Path $run.FullName 'response.json') -Encoding UTF8
$receipt = $response.Content | ConvertFrom-Json
if ($receipt.code -ne 0) { throw "Bridge returned $($receipt.code): $($receipt.message)" }
$results = @($receipt.data.results)
if ($results.Count -ne 1 -or $results[0].videoPath -ne $inputPath) {
    throw 'The Bridge result does not identify the submitted file.'
}
$result = $results[0]
if ($result.error) { throw "Transcription failed: $($result.error)" }
if ([string]::IsNullOrWhiteSpace($result.fullText) -or $result.segmentCount -lt 1 -or
    [string]::IsNullOrWhiteSpace($result.srtPath)) {
    throw 'The Bridge did not return speech text and a subtitle file.'
}

# This legacy route writes a unique SRT under the local service temp directory.
$expectedSrt = Join-Path ([IO.Path]::GetTempPath()) "bgxiong_video_subtitle/$runId.srt"
if ([IO.Path]::GetFullPath($result.srtPath) -ne [IO.Path]::GetFullPath($expectedSrt)) {
    throw 'The Bridge returned an unexpected subtitle path.'
}
$subtitle = Get-Content -Raw -Encoding UTF8 -LiteralPath $result.srtPath
$cuePattern = '(?m)^\d{2}:\d{2}:\d{2},\d{3} --> \d{2}:\d{2}:\d{2},\d{3}[ \t]*\r?\n\S'
$cueCount = [regex]::Matches($subtitle, $cuePattern).Count
if ($cueCount -eq 0) { throw 'The subtitle file contains no timed text.' }

$transcriptPath = Join-Path $run.FullName 'transcript.txt'
$srtPath = Join-Path $run.FullName 'subtitles.srt'
[IO.File]::WriteAllText($transcriptPath, $result.fullText, [Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath $result.srtPath -Destination $srtPath
[pscustomobject] @{
    instanceId = $registry.instanceId
    modelConfigId = $ModelConfigId
    requestMs = $timer.ElapsedMilliseconds
    serviceMs = $receipt.data.totalElapsedMs
    segmentCount = $result.segmentCount
    subtitleCueCount = $cueCount
    transcriptPath = $transcriptPath
    srtPath = $srtPath
} | ConvertTo-Json
