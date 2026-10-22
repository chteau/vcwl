# Installs vcwl on Windows:  irm https://vcwl.chteau.bzh/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$url = 'https://github.com/chteau/vcwl/releases/latest/download/vcwl-x86_64-pc-windows-msvc.zip'
$dir = Join-Path $env:LOCALAPPDATA 'vcwl\bin'
$zip = Join-Path $env:TEMP 'vcwl.zip'

New-Item -ItemType Directory -Force $dir | Out-Null
Write-Host "Downloading $url"
Invoke-WebRequest $url -OutFile $zip
Expand-Archive $zip $dir -Force
Remove-Item $zip
Unblock-File (Join-Path $dir 'vcwl.exe')

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($path -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', "$dir;$path".TrimEnd(';'), 'User')
}
$env:Path = "$dir;$env:Path"

Write-Host "Installed $(& (Join-Path $dir 'vcwl.exe') --version) to $dir"
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host 'Node.js 20+ is needed for vcwl run, vcwl test and prerendering: winget install OpenJS.NodeJS.LTS'
}
Write-Host 'Open a new terminal, then: vcwl new my-app; cd my-app; vcwl dev'
