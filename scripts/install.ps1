#Requires -Version 5.1
<#
.SYNOPSIS
  Ensure Hermes runtime dependencies (ffmpeg, node, ...).

.DESCRIPTION
  Thin wrapper around `hermes _ensure-dep` for gateway startup and installers.
  FFmpeg is downloaded from release mirrors with parallel latency probing (Rust core).

.PARAMETER Ensure
  Dependency name: ffmpeg | node | browser | ripgrep

.PARAMETER HermesHome
  Hermes home directory (defaults to $env:HERMES_HOME).

.PARAMETER Quiet
  Suppress non-error stdout.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Ensure,

    [string]$HermesHome = $env:HERMES_HOME,

    [switch]$Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-HermesBinary {
    $names = @(
        $env:HERMES_BIN,
        'hermes-agent-ultra',
        'hermes-ultra',
        'hermes'
    ) | Where-Object { $_ -and $_.Trim() }

    foreach ($name in $names) {
        if (Test-Path -LiteralPath $name -PathType Leaf) {
            return (Resolve-Path -LiteralPath $name).Path
        }
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) {
            return $cmd.Source
        }
    }

    $scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
    $repoRoot = Split-Path $scriptRoot -Parent
    $candidates = @(
        (Join-Path $repoRoot 'target\release\hermes-agent-ultra.exe'),
        (Join-Path $repoRoot 'target\debug\hermes-agent-ultra.exe'),
        (Join-Path $repoRoot 'target\release\hermes-ultra.exe'),
        (Join-Path $repoRoot 'target\debug\hermes-ultra.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }

    return $null
}

$bin = Resolve-HermesBinary
if (-not $bin) {
    Write-Error "Hermes binary not found on PATH; build or install hermes-agent-ultra first."
    exit 1
}

$args = @('_ensure-dep', $Ensure)
if ($Quiet) {
    $args += '--quiet'
}
if ($HermesHome) {
    $env:HERMES_HOME = $HermesHome
}

& $bin @args
exit $LASTEXITCODE
