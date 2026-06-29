param(
    [Parameter(Mandatory = $true)]
    [string]$ServiceName,
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command sc.exe -ErrorAction SilentlyContinue)) {
    throw "sc.exe not found"
}

sc.exe create $ServiceName binPath= "`"$BinaryPath`"" type= user start= auto
sc.exe description $ServiceName "Terra Hermes HTTP backend service"
sc.exe start $ServiceName
