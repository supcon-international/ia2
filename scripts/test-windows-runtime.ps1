# Native Windows runtime smoke test. Runs a device-free copy of sim_smoke
# on a temporary loopback port; never opens a fieldbus or physical serial port.
# Run after cargo build -p ia2-runtime. Logs/results remain in target/.
[CmdletBinding()]
param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot),
    [string]$RuntimePath,
    [string]$ArtifactsDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This test requires native Windows.' }
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
if (-not $RuntimePath) { $RuntimePath = Join-Path $RepoRoot 'target\debug\ia2-runtime.exe' }
$RuntimePath = (Resolve-Path -LiteralPath $RuntimePath).Path
if (-not $ArtifactsDirectory) {
    $ArtifactsDirectory = Join-Path $RepoRoot ('target\windows-runtime-smoke\' + [Guid]::NewGuid().ToString('N'))
}
$null = New-Item -ItemType Directory -Path $ArtifactsDirectory -Force
$ArtifactsDirectory = (Resolve-Path -LiteralPath $ArtifactsDirectory).Path
# Keep this script ASCII for Windows PowerShell 5.1, but exercise a real
# non-ASCII path (Chinese "simulation") plus a space in the child process.
$unicodeName = [string][char]0x6a21 + [string][char]0x62df + ' project'
$projectPath = Join-Path $ArtifactsDirectory $unicodeName
Copy-Item -LiteralPath (Join-Path $RepoRoot 'examples\sim_smoke') -Destination $projectPath -Recurse
if ((Test-Path -LiteralPath (Join-Path $projectPath 'devices')) -or
    (Test-Path -LiteralPath (Join-Path $projectPath 'northbound.toml'))) {
    throw 'sim_smoke must stay device-free and have no northbound connection for this test.'
}
$stdout = Join-Path $ArtifactsDirectory 'runtime.stdout.log'
$stderr = Join-Path $ArtifactsDirectory 'runtime.stderr.log'
$runtime = $null
$baseUrl = $null
$passed = $false

function Request-Runtime([string]$Method, [string]$Path, $Body = $null) {
    $options = @{
        Uri = $baseUrl + $Path
        Method = $Method
        TimeoutSec = 5
        Headers = @{ 'X-IA2-Origin' = 'windows-runtime-smoke' }
    }
    if ($null -ne $Body) {
        $options.ContentType = 'application/json'
        $options.Body = ConvertTo-Json -InputObject $Body -Compress
    }
    # Windows PowerShell 5.1 may guess an ANSI encoding when a JSON
    # response has no charset. Decode the response bytes as JSON's UTF-8.
    $response = Invoke-WebRequest @options -UseBasicParsing
    $text = [Text.Encoding]::UTF8.GetString($response.RawContentStream.ToArray())
    ConvertFrom-Json -InputObject $text
}

function Read-Level($Status) {
    $variable = @($Status.last_snapshot.vars | Where-Object { $_.name -eq 'level' })
    if ($variable.Count -ne 1 -or $variable[0].type_name -ne 'REAL') {
        throw 'Expected exactly one REAL variable named level in the runtime snapshot.'
    }
    # Decode raw VM bits, never parse locale-dependent display strings.
    [BitConverter]::ToSingle([BitConverter]::GetBytes([uint32]$variable[0].bits), 0)
}

try {
    $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    $baseUrl = 'http://127.0.0.1:' + $port
    $arguments = @('--project-dir', ('"{0}"' -f $projectPath), '--bind', ('127.0.0.1:' + $port),
        '--state-dir', ('"{0}"' -f (Join-Path $ArtifactsDirectory 'state')))
    $runtime = Start-Process -FilePath $RuntimePath -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Cache the process handle while it is alive: Windows PowerShell 5.1
    # otherwise may lose ExitCode for redirected Start-Process children.
    $null = $runtime.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $status = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $runtime.Refresh()
        if ($runtime.HasExited) { throw ('Runtime exited during startup. See ' + $stderr) }
        try { $status = Request-Runtime GET '/status' } catch { $status = $null }
        if ($null -ne $status -and $status.scan_count -gt 0) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $status -or $status.scan_count -eq 0) { throw 'Runtime never produced a scan snapshot.' }
    if ($status.project -ne 'sim_smoke' -or $status.mode.kind -ne 'running' -or
        $null -ne $status.fault -or $status.watchdog_tripped -or @($status.devices).Count -ne 0) {
        throw ('Unexpected startup status: ' + (ConvertTo-Json -InputObject $status -Depth 8 -Compress))
    }
    $system = Request-Runtime GET '/system'
    if ($system.os -ne 'windows' -or @($system.nics).Count -eq 0) { throw 'Native Windows NIC inventory is missing.' }
    $managedNics = @([Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() |
        Where-Object { $_.OperationalStatus -eq [Net.NetworkInformation.OperationalStatus]::Up })
    foreach ($nic in $managedNics) {
        if ($nic.Name -notin @($system.nics | ForEach-Object { $_.name })) {
            throw ('Native NIC inventory lost an active interface name: ' + $nic.Name)
        }
    }
    foreach ($serialPort in $system.serial_ports) {
        if ($serialPort -notmatch '^COM[0-9]+$') { throw ('Unexpected Windows serial-port name: ' + $serialPort) }
    }
    $system | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ArtifactsDirectory 'system.json') -Encoding UTF8

    $null = Request-Runtime POST '/pause'
    Start-Sleep -Milliseconds 250
    $paused = Request-Runtime GET '/status'
    Start-Sleep -Milliseconds 250
    $stillPaused = Request-Runtime GET '/status'
    if ($stillPaused.mode.kind -ne 'paused' -or $stillPaused.scan_count -ne $paused.scan_count) {
        throw 'Pause did not stop scan execution.'
    }
    $initialLevel = Read-Level $stillPaused
    $write = Request-Runtime POST '/write' @{ name = 'inlet_cmd'; value = 1 }
    if (-not $write.ok -or $write.value -ne 1) { throw 'Writing the simulated inlet command failed.' }
    $null = Request-Runtime POST '/step' @{ cycles = 4 }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        Start-Sleep -Milliseconds 100
        $stepped = Request-Runtime GET '/status'
    } while (($stepped.mode.kind -ne 'paused' -or $stepped.scan_count -lt ($stillPaused.scan_count + 4)) -and
        [DateTime]::UtcNow -lt $deadline)
    $steppedLevel = Read-Level $stepped
    if ($stepped.mode.kind -ne 'paused' -or $stepped.scan_count -ne ($stillPaused.scan_count + 4) -or
        [Math]::Abs($steppedLevel - $initialLevel - 2.0) -gt 0.0001) {
        throw ('Four simulated scans did not increase REAL level by 2: ' + (ConvertTo-Json -InputObject $stepped -Depth 8 -Compress))
    }
    $null = Request-Runtime POST '/write' @{ name = 'inlet_cmd'; value = 0 }
    $null = Request-Runtime POST '/resume'
    Start-Sleep -Milliseconds 400
    $resumed = Request-Runtime GET '/status'
    if ($resumed.mode.kind -ne 'running' -or $resumed.scan_count -le $stepped.scan_count -or
        (Read-Level $resumed) -ne $steppedLevel -or $null -ne $resumed.fault -or $resumed.watchdog_tripped) {
        throw 'Resume did not advance scans while preserving the closed-inlet level.'
    }
    $resumed | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ArtifactsDirectory 'status.json') -Encoding UTF8
    $null = Request-Runtime POST '/stop'
    if (-not $runtime.WaitForExit(15000)) { throw 'POST /stop did not terminate the runtime within 15 seconds.' }
    $runtime.Refresh()
    if ($runtime.ExitCode -ne 0) { throw ('Runtime shutdown failed with exit code ' + $runtime.ExitCode) }
    $passed = $true
    $report = [ordered]@{
        result = 'passed'
        os = $system.os
        arch = $system.arch
        nics = @($system.nics).Count
        serial_ports = @($system.serial_ports)
        non_ascii_path = $projectPath
        initial_scan_count = $stillPaused.scan_count
        step_scan_count = $stepped.scan_count
        resumed_scan_count = $resumed.scan_count
        level_after_four_scans = $steppedLevel
        clean_exit = $runtime.ExitCode
        artifacts = $ArtifactsDirectory
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ArtifactsDirectory 'result.json') -Encoding UTF8
    $report | ConvertTo-Json -Depth 8
} finally {
    if ($null -ne $runtime) {
        $runtime.Refresh()
        if (-not $runtime.HasExited) {
            try { $null = Request-Runtime POST '/stop' } catch { }
            if (-not $runtime.WaitForExit(10000)) { Stop-Process -Id $runtime.Id -Force }
        }
        $runtime.Dispose()
    }
    if (-not $passed) { Write-Warning ('Runtime smoke test failed; logs retained at ' + $ArtifactsDirectory) }
}
