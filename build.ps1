# Native Windows package: .\build.ps1
# Complete release: .\build.ps1 -All -WslDistro Ubuntu -WslUser root
[CmdletBinding()]
param(
    [switch]$All,
    [string]$DestinationRoot = 'dist',
    [string]$WslDistro = 'Ubuntu',
    [string]$WslUser = ''
)
$ErrorActionPreference = 'Stop'
$pythonCommand = Get-Command -Name python -CommandType Application -ErrorAction SilentlyContinue |
    Where-Object { $_.Source -notlike '*\WindowsApps\*' } | Select-Object -First 1
if (-not $pythonCommand) { throw 'Install Python 3.12+ and make python available on PATH.' }
$releaseArguments = @(
    (Join-Path -Path $PSScriptRoot -ChildPath 'tools/release_package.py'),
    'build', '--platform', $(if ($All) { 'all' } else { 'windows' }),
    '--destination-root', $DestinationRoot, '--wsl-distro', $WslDistro
)
if ($WslUser) { $releaseArguments += @('--wsl-user', $WslUser) }
& $pythonCommand.Source @releaseArguments
if ($LASTEXITCODE -ne 0) { throw "Release packaging failed (exit $LASTEXITCODE)." }
