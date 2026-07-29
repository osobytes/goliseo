[CmdletBinding()]
param(
    [string]$Python = "",
    [int]$Port = 8000,
    [string]$OutputRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
if (-not $OutputRoot) {
    $OutputRoot = Join-Path $ProjectRoot ".cache\omp0-windows-campaign\$Timestamp"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputRoot)) {
    $OutputRoot = [System.IO.Path]::GetFullPath((Join-Path $ProjectRoot $OutputRoot))
} else {
    $OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
}

$ProjectPrefix = $ProjectRoot.TrimEnd("\") + "\"
$CachePrefix = (Join-Path $ProjectRoot ".cache").TrimEnd("\") + "\"
if (
    $OutputRoot.StartsWith($ProjectPrefix, [System.StringComparison]::OrdinalIgnoreCase) -and
    -not $OutputRoot.StartsWith($CachePrefix, [System.StringComparison]::OrdinalIgnoreCase)
) {
    throw "OutputRoot inside the repository must be under the ignored .cache directory."
}
if (Test-Path -LiteralPath $OutputRoot) {
    throw "OutputRoot already exists; choose a new empty path to prevent stale evidence: $OutputRoot"
}

New-Item -ItemType Directory -Path $OutputRoot | Out-Null
$PacketRoot = Join-Path $OutputRoot "packets"
$ArchiveRoot = Join-Path $OutputRoot "archives"
$VenvRoot = Join-Path $OutputRoot ".venv"
$SeleniumCache = Join-Path $OutputRoot "selenium-cache"
$SummaryPath = Join-Path $OutputRoot "campaign-summary.json"
New-Item -ItemType Directory -Path $PacketRoot, $ArchiveRoot, $SeleniumCache | Out-Null

$env:SE_CACHE_PATH = $SeleniumCache
$env:SE_AVOID_STATS = "true"
$env:SE_SKIP_DRIVER_IN_PATH = "true"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath exited with code $LASTEXITCODE"
    }
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )
    $Value | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 $Path
}

function Read-Decision {
    param([Parameter(Mandatory = $true)][string]$Prompt)
    while ($true) {
        $Decision = (Read-Host "$Prompt (yes/no/unsure)").Trim().ToLowerInvariant()
        if ($Decision -in @("yes", "no", "unsure")) {
            return $Decision
        }
        Write-Warning "Enter exactly yes, no, or unsure."
    }
}

function Get-OnlyPacket {
    param([Parameter(Mandatory = $true)][string]$RowRoot)
    $Packets = @(Get-ChildItem -LiteralPath $RowRoot -Directory)
    if ($Packets.Count -ne 1) {
        throw "Expected exactly one new packet under $RowRoot; found $($Packets.Count)."
    }
    return $Packets[0]
}

function Get-FileEvidence {
    param([Parameter(Mandatory = $true)][string]$Path)
    $Item = Get-Item -LiteralPath $Path
    if ($Item.Length -le 0) {
        throw "Evidence file is empty: $Path"
    }
    return [ordered]@{
        name = $Item.Name
        path = $Item.FullName
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
        size_bytes = $Item.Length
    }
}

if ($Python) {
    $BootstrapPython = $Python
    $BootstrapArguments = @()
} elseif (Get-Command "py.exe" -ErrorAction SilentlyContinue) {
    $BootstrapPython = "py.exe"
    $BootstrapArguments = @("-3")
} elseif (Get-Command "python.exe" -ErrorAction SilentlyContinue) {
    $BootstrapPython = "python.exe"
    $BootstrapArguments = @()
} else {
    $BootstrapPython = $null
    $BootstrapArguments = @()
}

$Server = $null
$LocationPushed = $false
$CampaignRows = @()
$CampaignExitCode = 0
$CampaignFailure = $null
$CleanupFailure = $null
$FirefoxHeap = $null

try {
    Push-Location $ProjectRoot
    $LocationPushed = $true
    if (-not $BootstrapPython) {
        throw "Python 3.11 or newer was not found (expected py.exe or python.exe)."
    }
    & $BootstrapPython @BootstrapArguments "-c" (
        "import sys; assert sys.version_info >= (3, 11), sys.version"
    )
    if ($LASTEXITCODE -ne 0) {
        throw "Python 3.11 or newer is required."
    }
    & $BootstrapPython @BootstrapArguments "-m" "venv" $VenvRoot
    if ($LASTEXITCODE -ne 0) {
        throw "Could not create the evidence-only Python environment."
    }

    $VenvPython = Join-Path $VenvRoot "Scripts\python.exe"
    Invoke-Checked $VenvPython @(
        "-m", "pip", "install", "--disable-pip-version-check", "--require-hashes",
        "--requirement", "scripts\browser_matrix-requirements.txt"
    )
    Invoke-Checked $VenvPython @("scripts\web_build.py", "--output", "build\web")

    $ServerStdout = Join-Path $OutputRoot "web-server.stdout.log"
    $ServerStderr = Join-Path $OutputRoot "web-server.stderr.log"
    $Server = Start-Process -FilePath $VenvPython -ArgumentList @(
        "scripts\web_serve.py", "build\web", "--host", "127.0.0.1", "--port", "$Port"
    ) -WorkingDirectory $ProjectRoot -PassThru -NoNewWindow `
        -RedirectStandardOutput $ServerStdout -RedirectStandardError $ServerStderr

    $BaseUrl = "http://127.0.0.1:$Port/"
    $Ready = $false
    for ($Attempt = 0; $Attempt -lt 60; $Attempt += 1) {
        if ($Server.HasExited) {
            throw "The artifact server exited before becoming ready; inspect $ServerStderr"
        }
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "${BaseUrl}manifest.json" -TimeoutSec 2 |
                Out-Null
            $Ready = $true
            break
        } catch {
            Start-Sleep -Milliseconds 250
        }
    }
    if (-not $Ready) {
        throw "The artifact server did not become ready at $BaseUrl"
    }

    Write-Host ""
    Write-Host "ATTENDED CAMPAIGN"
    Write-Host "Keep this unlocked hardware-accelerated desktop foregrounded at 100% scaling."
    Write-Host "Each browser/viewport uses a fresh profile. During Match, listen and press"
    Write-Host "physical standard-mapped A then B. Include about 45 minutes for six rows"
    Write-Host "and the attended Firefox heap-snapshot companion."
    Write-Host ""

    foreach ($Browser in @("chrome", "firefox")) {
        foreach ($Viewport in @("960x540", "1280x720", "1920x1080")) {
            $StabilitySeconds = if ($Viewport -eq "960x540") { 600 } else { 0 }
            Read-Host (
                "Ready for $Browser $Viewport with audible speakers and the physical " +
                "controller connected? Press Enter"
            ) | Out-Null

            $RowRoot = Join-Path $PacketRoot "$Browser-$Viewport"
            if (Test-Path -LiteralPath $RowRoot) {
                throw "Refusing reused row output: $RowRoot"
            }
            $MatrixArguments = @(
                "scripts\browser_matrix.py",
                "--browser", $Browser,
                "--url", $BaseUrl,
                "--viewport", $Viewport,
                "--flow-timeout", "300",
                "--stability-seconds", "$StabilitySeconds",
                "--require-gamepad",
                "--output", $RowRoot
            )
            $RunMemoryCompanion = (
                $Browser -eq "firefox" -and
                $Viewport -eq "960x540" -and
                $StabilitySeconds -gt 0
            )
            if ($RunMemoryCompanion) {
                $MatrixArguments += "--external-memory-companion"
            }

            & $VenvPython @MatrixArguments
            $MatrixExitCode = $LASTEXITCODE
            $Packet = Get-OnlyPacket $RowRoot
            $AudioHeard = Read-Decision "Was GOLISEO audio audible during $Browser $Viewport?"
            $PhysicalButtons = Read-Decision (
                "Did you press physical standard-mapped A then B during Match in " +
                "$Browser $Viewport?"
            )
            $OperatorPass = $AudioHeard -eq "yes" -and $PhysicalButtons -eq "yes"
            $RowPass = $MatrixExitCode -eq 0 -and $OperatorPass
            if (-not $RowPass) {
                $CampaignExitCode = 1
            }
            $OperatorPath = Join-Path $Packet.FullName "operator-observation.json"
            Write-JsonFile $OperatorPath ([ordered]@{
                audio_audible = $AudioHeard
                browser = $Browser
                captured_at = (Get-Date).ToUniversalTime().ToString("o")
                pass = $OperatorPass
                physical_standard_mapped_a_then_b = $PhysicalButtons
                viewport = $Viewport
            })
            $CampaignRows += [ordered]@{
                archive = $null
                archive_sha256 = $null
                browser = $Browser
                matrix_exit_code = $MatrixExitCode
                operator_audio_audible = $AudioHeard
                operator_physical_a_then_b = $PhysicalButtons
                packet = $Packet.FullName
                pass = $RowPass
                stability_seconds = $StabilitySeconds
                viewport = $Viewport
            }
            Write-Host "Raw packet: $($Packet.FullName)"
        }
    }

    $FirefoxLongRow = $CampaignRows |
        Where-Object { $_.browser -eq "firefox" -and $_.viewport -eq "960x540" } |
        Select-Object -First 1
    if (-not $FirefoxLongRow) {
        throw "The Firefox 960x540 packet is required before heap capture."
    }
    $FirefoxPacketSummary = Get-Content -Raw -LiteralPath (
        Join-Path $FirefoxLongRow.packet "summary.json"
    ) |
        ConvertFrom-Json
    if (
        $FirefoxPacketSummary.memory_companion.status -ne "pass" -or
        -not $FirefoxPacketSummary.memory_companion.summary
    ) {
        throw "The Firefox 960x540 packet did not complete its attended heap companion."
    }
    $FirefoxHeap = Get-Content -Raw -LiteralPath (
        [string]$FirefoxPacketSummary.memory_companion.summary
    ) |
        ConvertFrom-Json

    foreach ($Row in $CampaignRows) {
        $Packet = Get-Item -LiteralPath $Row.packet
        $Archive = Join-Path $ArchiveRoot "$($Packet.Name).zip"
        Compress-Archive -Path (Join-Path $Packet.FullName "*") -DestinationPath $Archive
        $ArchiveHashRecord = Get-FileHash -Algorithm SHA256 -LiteralPath $Archive
        $ArchiveHash = $ArchiveHashRecord.Hash.ToLowerInvariant()
        $Row.archive = $Archive
        $Row.archive_sha256 = $ArchiveHash
        Write-Host "Archive: $Archive"
        Write-Host "Archive SHA-256: $ArchiveHash"
    }
} catch {
    $CampaignExitCode = 1
    $CampaignFailure = [ordered]@{
        message = $_.Exception.Message
        script_stack_trace = $_.ScriptStackTrace
        type = $_.Exception.GetType().FullName
    }
} finally {
    if ($Server -and -not $Server.HasExited) {
        Stop-Process -Id $Server.Id -ErrorAction SilentlyContinue
        if (-not $Server.WaitForExit(10000)) {
            $CampaignExitCode = 1
            $CleanupFailure = "Artifact server did not exit within 10 seconds."
        }
    }
    if ($LocationPushed) {
        Pop-Location
    }
    $ExpectedRows = 6
    $CampaignComplete = (
        $CampaignExitCode -eq 0 -and
        $null -eq $CampaignFailure -and
        $null -eq $CleanupFailure -and
        $CampaignRows.Count -eq $ExpectedRows -and
        @($CampaignRows | Where-Object { $_.pass -ne $true }).Count -eq 0 -and
        $null -ne $FirefoxHeap -and
        $FirefoxHeap.pass -eq $true
    )
    if (-not $CampaignComplete) {
        $CampaignExitCode = 1
    }
    Write-JsonFile $SummaryPath ([ordered]@{
        captured_at = (Get-Date).ToUniversalTime().ToString("o")
        cleanup_failure = $CleanupFailure
        complete = $CampaignComplete
        failure = $CampaignFailure
        firefox_heap = $FirefoxHeap
        output_root = $OutputRoot
        requirements_lock = Get-FileEvidence (
            Join-Path $ProjectRoot "scripts\browser_matrix-requirements.txt"
        )
        rows = $CampaignRows
        selenium_cache = $SeleniumCache
        server_stopped = ($null -eq $Server -or $Server.HasExited)
    })
}

Write-Host ""
Write-Host "Campaign summary: $SummaryPath"
Write-Host "Raw packets: $PacketRoot"
Write-Host "Archives: $ArchiveRoot"
if ($CampaignExitCode -ne 0) {
    Write-Warning (
        "The campaign is incomplete. A failed/unsure operator answer, missing heap/GPU " +
        "evidence, matrix failure, or cleanup failure must not be inferred as a pass."
    )
}
exit $CampaignExitCode
