#requires -Version 5.1
<#
Windows per-user installer for IA2 and industrial-automation-skill.
Run from a source checkout, or from an extracted package-windows.ps1 ZIP.
No elevation, symlinks, PATH/profile edits, or Windows service are required.
For macOS/Linux use install-skill.sh; agent discovery is checked by
check-windows.ps1 -AdaptationOnly and check-agent-adaptation.sh.
#>
[CmdletBinding()]
param(
    [switch]$SkillOnly,
    [switch]$SkipBuild,
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA 'IA2'),
    [string]$ClaudeDir = (Join-Path $env:USERPROFILE '.claude'),
    [string]$AgentsDir = (Join-Path $env:USERPROFILE '.agents'),
    [switch]$NoShortcuts
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Run this installer in native Windows PowerShell.' }
$SourceRoot = Split-Path $PSScriptRoot -Parent
# Resolve relative paths against PowerShell's location, not the process CWD.
$InstallRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallRoot)
$ClaudeDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ClaudeDir)
$AgentsDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($AgentsDir)
$SkillName = 'industrial-automation-skill'
$SkillSource = Join-Path $SourceRoot ".claude\skills\$SkillName"
$IsPackage = Test-Path -LiteralPath (Join-Path $SourceRoot 'windows-package.json')
$AppMarker = '.ia2-install'
$SkillMarker = '.ia2-installed-skill'

function Invoke-Native([string]$Program, [string[]]$Arguments) {
    Get-Command -Name $Program -ErrorAction Stop | Out-Null
    $savedPreference = $ErrorActionPreference
    try {
        # PowerShell 5.1 turns captured native stderr into error records;
        # retain that output, but use the actual process exit code.
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments
        $nativeExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $savedPreference }
    if ($nativeExit -ne 0) { throw "$Program failed with exit code $nativeExit" }
}

function Assert-Destination([string]$Destination, [string]$Source, [string]$Marker) {
    $dest = [IO.Path]::GetFullPath($Destination).TrimEnd('\', '/')
    $src = [IO.Path]::GetFullPath($Source).TrimEnd('\', '/')
    if ($dest -eq $src -or $src.StartsWith($dest + '\', [StringComparison]::OrdinalIgnoreCase) -or $dest.StartsWith($src + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Install destination overlaps its source: $Destination"
    }
    # Reject aliases through junctions/symlinks before replacing any tree.
    $cursor = $dest
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Install destination traverses a junction or symlink: $cursor"
            }
        }
        $parent = Split-Path $cursor -Parent
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    if (Test-Path -LiteralPath $dest) {
        if (-not (Test-Path -LiteralPath $dest -PathType Container)) { throw "Not a directory: $dest" }
        $items = @(Get-ChildItem -LiteralPath $dest -Force)
        if ($items.Count -gt 0 -and -not (Test-Path -LiteralPath (Join-Path $dest $Marker))) {
            throw "Existing directory is not owned by this installer; leaving it untouched: $dest"
        }
    }
}

function Install-Tree([string]$Staged, [string]$Destination) {
    $backup = "$Destination.previous-$([guid]::NewGuid().ToString('N'))"
    $hadPrevious = Test-Path -LiteralPath $Destination
    if ($hadPrevious) { Move-Item -LiteralPath $Destination -Destination $backup }
    try { Move-Item -LiteralPath $Staged -Destination $Destination }
    catch {
        if ($hadPrevious) { Move-Item -LiteralPath $backup -Destination $Destination }
        throw
    }
    if ($hadPrevious) { Remove-Item -LiteralPath $backup -Recurse -Force }
}

$SkillTargets = @((Join-Path $ClaudeDir "skills\$SkillName"), (Join-Path $AgentsDir "skills\$SkillName"))
if (-not (Test-Path -LiteralPath (Join-Path $SkillSource 'SKILL.md'))) { throw "Skill source missing: $SkillSource" }
foreach ($destination in $SkillTargets) { Assert-Destination $destination $SkillSource $SkillMarker }
$firstSkill = $SkillTargets[0].TrimEnd('\')
$secondSkill = $SkillTargets[1].TrimEnd('\')
if ($firstSkill -eq $secondSkill -or $firstSkill.StartsWith($secondSkill + '\', [StringComparison]::OrdinalIgnoreCase) -or $secondSkill.StartsWith($firstSkill + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ClaudeDir and AgentsDir skill destinations must not overlap.'
}
if (-not $SkillOnly) {
    Assert-Destination $InstallRoot $SourceRoot $AppMarker
    foreach ($destination in $SkillTargets) {
        $app = [IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
        $skill = [IO.Path]::GetFullPath($destination).TrimEnd('\')
        if ($skill -eq $app -or $skill.StartsWith($app + '\', [StringComparison]::OrdinalIgnoreCase) -or $app.StartsWith($skill + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Application and skill installation directories must not overlap.'
        }
    }
    # Moving a running Windows executable fails; report before changing skills.
    foreach ($process in @(Get-Process ia2-server,cs,ia2-runtime,lsp-launcher -ErrorAction SilentlyContinue)) {
        if ($process.Path -and $process.Path.StartsWith([IO.Path]::GetFullPath($InstallRoot).TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "Close installed IA2 processes before upgrading (PID $($process.Id)): $($process.Path)"
        }
    }
    if (-not $SkipBuild -and -not $IsPackage) {
        Push-Location $SourceRoot
        try {
            Invoke-Native git @('submodule', 'update', '--init', '--recursive')
            Invoke-Native pnpm @('install', '--frozen-lockfile')
            Invoke-Native cargo @('test', '--locked', '-p', 'server')
            & (Join-Path $PSScriptRoot 'build-windows.ps1')
            Invoke-Native pnpm @('--filter', '@cs/web', 'build')
        } finally { Pop-Location }
    }
    $BinarySource = if ($IsPackage) { Join-Path $SourceRoot 'bin' } else { Join-Path $SourceRoot 'target\x86_64-pc-windows-msvc\release' }
    $WebSource = if ($IsPackage) { Join-Path $SourceRoot 'web' } else { Join-Path $SourceRoot 'apps\web\dist' }
    $BinaryNames = [ordered]@{ 'cs.exe' = 'cs.exe'; 'server.exe' = 'ia2-server.exe'; 'lsp-launcher.exe' = 'lsp-launcher.exe'; 'ia2-runtime.exe' = 'ia2-runtime.exe' }
    foreach ($name in $BinaryNames.Keys) {
        $inputName = if ($IsPackage) { $BinaryNames[$name] } else { $name }
        if (-not (Test-Path -LiteralPath (Join-Path $BinarySource $inputName))) { throw "Required binary missing: $inputName in $BinarySource" }
    }
    foreach ($required in @((Join-Path $WebSource 'index.html'), (Join-Path $WebSource 'hmi.html'), (Join-Path $SourceRoot 'library'))) {
        if (-not (Test-Path -LiteralPath $required)) { throw "Required artifact missing: $required" }
    }
}

$stagedDirectories = @()
try {
    # Stage all content before replacing any installed directory.
    $skillStages = @()
    foreach ($destination in $SkillTargets) {
        New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
        $stage = "$destination.staged-$([guid]::NewGuid().ToString('N'))"
        Copy-Item -LiteralPath $SkillSource -Destination $stage -Recurse -Force
        Set-Content -LiteralPath (Join-Path $stage $SkillMarker) -Value 'IA2 Windows skill installer v1' -Encoding ASCII
        $stagedDirectories += $stage
        $skillStages += $stage
    }
    if (-not $SkillOnly) {
        New-Item -ItemType Directory -Force -Path (Split-Path ([IO.Path]::GetFullPath($InstallRoot)) -Parent) | Out-Null
        $appStage = "$InstallRoot.staged-$([guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Directory -Force -Path (Join-Path $appStage 'bin') | Out-Null
        $stagedDirectories += $appStage
        foreach ($name in $BinaryNames.Keys) {
            $inputName = if ($IsPackage) { $BinaryNames[$name] } else { $name }
            Copy-Item -LiteralPath (Join-Path $BinarySource $inputName) -Destination (Join-Path $appStage "bin\$($BinaryNames[$name])")
        }
        Copy-Item -LiteralPath $WebSource -Destination (Join-Path $appStage 'web') -Recurse -Force
        Copy-Item -LiteralPath (Join-Path $SourceRoot 'library') -Destination (Join-Path $appStage 'library') -Recurse -Force
        Set-Content -LiteralPath (Join-Path $appStage $AppMarker) -Value 'IA2 Windows installer v1; projects and preferences are outside this directory.' -Encoding ASCII
        $launcher = @'
#requires -Version 5.1
[CmdletBinding()]
param([switch]$Terminal, [ValidateRange(1024,65535)][int]$Port = 3001)
$ErrorActionPreference = 'Stop'
$env:PATH = (Join-Path $PSScriptRoot 'bin') + ';' + $env:PATH
$env:IA2_LIBRARY_DIR = Join-Path $PSScriptRoot 'library'
Set-Location $env:USERPROFILE
if ($Terminal) {
    Write-Host 'IA2 terminal. cs, ia2-server and ia2-runtime are on PATH in this window only.'
    Write-Host ('Start the IDE in another window with: & "' + (Join-Path $PSScriptRoot 'IA2.ps1') + '"')
    return
}
Write-Host "IA2 IDE: http://127.0.0.1:$Port"
Write-Host 'Keep this window open. Ctrl+C stops this server; closing only the browser does not stop it.'
Write-Host 'Open the URL after the server prints its listening address. Startup errors remain visible below.'
$server = Join-Path $PSScriptRoot 'bin\ia2-server.exe'
Get-Command -Name $server -ErrorAction Stop | Out-Null
try {
    $ErrorActionPreference = 'Continue'
    & $server --bind "127.0.0.1:$Port" --static-dir (Join-Path $PSScriptRoot 'web') --library-dir $env:IA2_LIBRARY_DIR
    $nativeExit = $LASTEXITCODE
} finally { $ErrorActionPreference = 'Stop' }
if ($nativeExit -ne 0) { throw "IA2 server exited with code $nativeExit" }
'@
        Set-Content -LiteralPath (Join-Path $appStage 'IA2.ps1') -Value $launcher -Encoding UTF8
        Invoke-Native (Join-Path $appStage 'bin\cs.exe') @('--version')
        Invoke-Native (Join-Path $appStage 'bin\ia2-server.exe') @('--help')
        Invoke-Native (Join-Path $appStage 'bin\ia2-runtime.exe') @('--help')
        Install-Tree $appStage $InstallRoot
    }
    for ($i = 0; $i -lt $SkillTargets.Count; $i++) { Install-Tree $skillStages[$i] $SkillTargets[$i] }
    if (-not $SkillOnly -and -not $NoShortcuts) {
        $menu = Join-Path ([Environment]::GetFolderPath('Programs')) 'IA2'
        New-Item -ItemType Directory -Force -Path $menu | Out-Null
        $shell = New-Object -ComObject WScript.Shell
        foreach ($name in @('IA2 IDE', 'IA2 Terminal')) {
            $link = $shell.CreateShortcut((Join-Path $menu "$name.lnk"))
            $link.TargetPath = Join-Path $PSHOME 'powershell.exe'
            $link.Arguments = '-NoProfile -NoExit -ExecutionPolicy Bypass -File "' + (Join-Path ([IO.Path]::GetFullPath($InstallRoot)) 'IA2.ps1') + '"'
            if ($name -eq 'IA2 Terminal') { $link.Arguments += ' -Terminal' }
            $link.WorkingDirectory = $env:USERPROFILE
            $link.Save()
        }
    }
} finally {
    foreach ($stage in $stagedDirectories) { if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force } }
}
Write-Host 'Installed industrial-automation-skill in both user discovery directories. Restart the coding agent to discover it.'
if (-not $SkillOnly) {
    Write-Host "Installed IA2 in $InstallRoot. Projects and preferences were preserved; PATH was not changed."
    Write-Host ('Start: & "' + (Join-Path $InstallRoot 'IA2.ps1') + '"')
    Write-Host 'Or open IA2 IDE / IA2 Terminal from the Start menu (unless -NoShortcuts was used).'
}
