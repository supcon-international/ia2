#requires -Version 5.1
<#
Native Windows fieldbus negative tests; no physical bus is selected.
Only malformed selectors, a reserved USB VID/PID with a nonexistent
serial, and the zero NIC GUID are used. No driver is installed.
Pass -RequireNpcapAbsent to prove startup without Npcap runtime DLLs.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot),
    [string]$RuntimePath,
    [string]$ArtifactsDirectory,
    [switch]$RequireNpcapAbsent
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitProcess) {
    throw 'Run this test in native 64-bit Windows PowerShell.'
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
if (-not $RuntimePath) { $RuntimePath = Join-Path $RepoRoot 'target\debug\ia2-runtime.exe' }
$RuntimePath = (Resolve-Path -LiteralPath $RuntimePath).Path
if (-not $ArtifactsDirectory) {
    $ArtifactsDirectory = Join-Path $RepoRoot ('target\windows-fieldbus-smoke\' + [Guid]::NewGuid().ToString('N'))
}
$ArtifactsDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ArtifactsDirectory)
if ((Test-Path -LiteralPath $ArtifactsDirectory) -and
    @(Get-ChildItem -LiteralPath $ArtifactsDirectory -Force).Count -ne 0) {
    throw 'ArtifactsDirectory must be new or empty; earlier evidence will not be overwritten.'
}
$null = New-Item -ItemType Directory -Path $ArtifactsDirectory -Force
$utf8 = [Text.UTF8Encoding]::new($false)
# ASCII script source remains readable by PowerShell 5.1. Exercise a
# Chinese path ("simulation") and spaces without relying on script BOMs.
$projectName = [string][char]0x6a21 + [string][char]0x62df + ' fieldbus errors'
$projectPath = Join-Path $ArtifactsDirectory $projectName
$stdout = Join-Path $ArtifactsDirectory 'runtime.stdout.log'
$stderr = Join-Path $ArtifactsDirectory 'runtime.stderr.log'
$runtime = $null
$baseUrl = $null
$passed = $false
$report = [ordered]@{
    result = 'failed'
    scope = 'device-free negative tests; physical CAN/EtherCAT exchange not tested'
    runtime = $RuntimePath
    runtime_sha256 = (Get-FileHash -LiteralPath $RuntimePath -Algorithm SHA256).Hash
    require_npcap_absent = [bool]$RequireNpcapAbsent
    artifacts = $ArtifactsDirectory
    forced_cleanup = $false
}

function Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Write-Json([string]$Name, $Value) {
    [IO.File]::WriteAllText((Join-Path $ArtifactsDirectory $Name),
        (ConvertTo-Json -InputObject $Value -Depth 12), $utf8)
}

function Request-Runtime([string]$Method, [string]$Path) {
    $response = Invoke-WebRequest -Uri ($baseUrl + $Path) -Method $Method -TimeoutSec 3 `
        -Headers @{ 'X-IA2-Origin' = 'windows-fieldbus-negative' } -UseBasicParsing
    # JSON is UTF-8, including when an HTTP Content-Type omits charset.
    $decoded = ConvertFrom-Json -InputObject ([Text.Encoding]::UTF8.GetString($response.RawContentStream.ToArray()))
    # PowerShell 5.1 preserves a top-level JSON array as one pipeline
    # object; explicitly enumerate it so discovery counts its devices.
    Write-Output $decoded
}

function Assert-UnhealthyDevices($Entries, [string]$Source) {
    $items = @($Entries)
    Assert ($items.Count -eq $cases.Count) "$Source did not report all four configured devices."
    foreach ($case in $cases) {
        $item = @($items | Where-Object { $_.name -eq $case.Name })
        Assert ($item.Count -eq 1) "$Source lost or duplicated $($case.Name)."
        Assert ($item[0].healthy -is [bool] -and -not $item[0].healthy) "$Source incorrectly marks $($case.Name) healthy."
    }
}

# Fixed fixtures, deliberately not parameters: callers cannot turn this
# regression into a physical bus test. No PDO channels or iomap outputs.
$cases = @(
    @{ Name = 'can_bad_selector'; Protocol = 'canopen'; Selector = 'gs_usb:invalid';
       ErrorPattern = 'gs_usb: expected gs_usb:<vid>:<pid>:<serial>:<channel>' },
    @{ Name = 'can_missing_adapter'; Protocol = 'canopen'; Selector = 'gs_usb:ffff:ffff:IA2-NO-SUCH-DEVICE:0';
       ErrorPattern = 'gs_usb: no matching USB CAN adapter' },
    @{ Name = 'ec_bad_selector'; Protocol = 'ethercat'; Selector = 'rpcap://IA2-INVALID';
       ErrorPattern = 'Windows EtherCAT NIC must be a local Ethernet alias|Npcap is unavailable at' },
    @{ Name = 'ec_missing_adapter'; Protocol = 'ethercat'; Selector = '{00000000-0000-0000-0000-000000000000}';
       ErrorPattern = 'Npcap is unavailable at|Npcap (?:create|activate) .*00000000-0000-0000-0000-000000000000' }
)

try {
    # Read only fixed DLL file metadata, never load a driver or enumerate NICs.
    $npcapFiles = @('Npcap\wpcap.dll', 'Npcap\Packet.dll', 'wpcap.dll', 'Packet.dll') |
        ForEach-Object {
            $path = Join-Path ([Environment]::SystemDirectory) $_
            [pscustomobject]@{ path = $path; present = (Test-Path -LiteralPath $path -PathType Leaf) }
        }
    $report['npcap_dll_files'] = @($npcapFiles)
    if ($RequireNpcapAbsent) {
        Assert (@($npcapFiles | Where-Object { $_.present }).Count -eq 0) `
            'RequireNpcapAbsent requested, but a Npcap/WinPcap runtime DLL exists. Nothing was removed.'
    }

    $fixture = Join-Path $RepoRoot 'examples\sim_smoke'
    foreach ($forbidden in @('devices', 'northbound.toml')) {
        Assert (-not (Test-Path -LiteralPath (Join-Path $fixture $forbidden))) `
            "sim_smoke must remain device-free: unexpected $forbidden."
    }
    Assert ([string]::IsNullOrWhiteSpace([IO.File]::ReadAllText((Join-Path $fixture 'iomap.toml')))) `
        'sim_smoke iomap must remain empty for this device-free test.'
    Copy-Item -LiteralPath $fixture -Destination $projectPath -Recurse
    $devicesPath = Join-Path $projectPath 'devices'
    $null = New-Item -ItemType Directory -Path $devicesPath
    foreach ($case in $cases) {
        $content = @(('name = "{0}"' -f $case.Name), ('protocol = "{0}"' -f $case.Protocol))
        if ($case.Protocol -eq 'canopen') {
            $content += @(('interface = "{0}"' -f $case.Selector), 'node_id = 1',
                'bitrate = 500000', 'start_on_connect = false', 'channels = []')
        } else {
            $content += @(('nic = "{0}"' -f $case.Selector), 'cycle_us = 10000',
                'dc_sync = "off"', 'slaves = []', 'channels = []')
        }
        [IO.File]::WriteAllText((Join-Path $devicesPath ($case.Name + '.toml')),
            (($content -join "`n") + "`n"), $utf8)
    }

    $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        $port = $listener.LocalEndpoint.Port
    } finally { $listener.Stop() }
    $baseUrl = 'http://127.0.0.1:' + $port
    $report['url'] = $baseUrl
    $arguments = @('--project-dir', ('"{0}"' -f $projectPath),
        '--bind', ('127.0.0.1:' + $port),
        '--state-dir', ('"{0}"' -f (Join-Path $ArtifactsDirectory 'state')))
    $runtime = Start-Process -FilePath $RuntimePath -ArgumentList $arguments -PassThru -NoNewWindow `
        -WorkingDirectory $ArtifactsDirectory -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Preserve ExitCode for redirected PowerShell 5.1 child processes.
    $null = $runtime.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(40)
    $status = $null
    $discovery = @()
    while ([DateTime]::UtcNow -lt $deadline) {
        $runtime.Refresh()
        if ($runtime.HasExited) { throw "Runtime exited during startup (exit $($runtime.ExitCode)); see $stderr." }
        try {
            $status = Request-Runtime GET '/status'
            $discovery = @(Request-Runtime GET '/discover')
        } catch { $status = $null }
        if ($null -ne $status -and $status.scan_count -gt 0 -and $discovery.Count -eq $cases.Count) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert ($null -ne $status -and $status.scan_count -gt 0) 'Runtime did not start scanning within 40 seconds.'
    Write-Json 'discover.json' $discovery
    Write-Json 'status.json' $status
    Assert ($status.project -eq 'sim_smoke' -and $status.mode.kind -eq 'running' -and $null -eq $status.fault) `
        'Fieldbus connection errors unexpectedly faulted the runtime.'
    Assert ($discovery.Count -eq $cases.Count) 'Discovery never reported all four negative fixtures.'
    Assert (@($status.devices).Count -eq $cases.Count) 'Status lost configured devices.'
    foreach ($case in $cases) {
        Assert ($case.Name -in @($status.devices)) "Status omitted $($case.Name)."
        $device = @($discovery | Where-Object { $_.name -eq $case.Name })
        Assert ($device.Count -eq 1) "Discovery lost or duplicated $($case.Name)."
        Assert ($device[0].protocol -eq $case.Protocol) "Wrong protocol for $($case.Name)."
        Assert ($device[0].connected -is [bool] -and -not $device[0].connected) `
            "$($case.Name) unexpectedly connected or silently selected simulation."
        Assert (@($device[0].slaves).Count -eq 0) "$($case.Name) fabricated discovered slaves."
        Assert (-not [string]::IsNullOrWhiteSpace($device[0].error) -and $device[0].error -match $case.ErrorPattern) `
            "Wrong or missing error for $($case.Name): $($device[0].error)"
        if ($RequireNpcapAbsent -and $case.Protocol -eq 'ethercat') {
            Assert ($device[0].error -match 'Npcap is unavailable at' -and
                $device[0].error -match 'install the official Npcap Windows driver') `
                "$($case.Name) did not report the expected actionable missing-Npcap error."
        }
    }
    Assert-UnhealthyDevices $status.device_health '/status'
    $health = Request-Runtime GET '/health'
    Write-Json 'health.json' $health
    Assert ($health.status -eq 'ok') 'Runtime liveness failed despite serving HTTP.'
    Assert ($health.fieldbus_healthy -is [bool] -and -not $health.fieldbus_healthy) `
        '/health incorrectly reports fieldbus_healthy=true.'
    Assert-UnhealthyDevices $health.devices '/health'

    Start-Sleep -Milliseconds 350
    $later = Request-Runtime GET '/status'
    Assert ($later.scan_count -gt $status.scan_count -and $null -eq $later.fault) `
        'Runtime stopped scanning after reporting disconnected fieldbuses.'
    Assert-UnhealthyDevices $later.device_health '/status after another scan'
    $report['devices'] = $discovery
    $report['initial_scan_count'] = $status.scan_count
    $report['later_scan_count'] = $later.scan_count
    $report['fieldbus_healthy'] = $health.fieldbus_healthy
    $null = Request-Runtime POST '/stop'
    Assert ($runtime.WaitForExit(15000)) 'POST /stop did not terminate the runtime within 15 seconds.'
    $runtime.Refresh()
    Assert ($runtime.ExitCode -eq 0) "Runtime stop returned exit $($runtime.ExitCode)."
    $report['clean_exit'] = $runtime.ExitCode
    $report['result'] = 'passed'
    $passed = $true
} catch {
    $report['error'] = $_.Exception.Message
    throw
} finally {
    if ($null -ne $runtime) {
        try {
            $runtime.Refresh()
            if (-not $runtime.HasExited) {
                try { $null = Request-Runtime POST '/stop' } catch { }
                if (-not $runtime.WaitForExit(5000)) {
                    $report['forced_cleanup'] = $true
                    Stop-Process -Id $runtime.Id -Force
                    $null = $runtime.WaitForExit(5000)
                }
            }
        } catch {
            $report['cleanup_error'] = $_.Exception.Message
            $report['result'] = 'failed'
            $passed = $false
        } finally { $runtime.Dispose() }
    }
    Write-Json 'result.json' $report
    if (-not $passed) { Write-Warning "Fieldbus negative test failed; evidence retained at $ArtifactsDirectory" }
}
Assert $passed 'Fieldbus negative test cleanup failed; see result.json.'
ConvertTo-Json -InputObject $report -Depth 12
