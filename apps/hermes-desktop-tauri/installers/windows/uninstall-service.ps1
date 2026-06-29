param(
    [Parameter(Mandatory = $true)]
    [string]$ServiceName,
    [string]$LogDir = "$env:LOCALAPPDATA\Terra\logs"
)

$ErrorActionPreference = "Stop"

sc.exe stop $ServiceName 2>$null
sc.exe delete $ServiceName 2>$null

if (Test-Path $LogDir) {
    Remove-Item -Recurse -Force $LogDir
}
