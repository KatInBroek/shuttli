param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$commit = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source commit' }
if (& git -C $root status --porcelain --untracked-files=normal) { throw 'Commit source changes before packaging' }
$version = [regex]::Match((Get-Content (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
if (!$OutputDirectory) { $OutputDirectory = Join-Path $root 'target\windows-package' }
$release = Join-Path $root 'target\release'
$stage = Join-Path $OutputDirectory 'Shuttli-windows-x64'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
foreach ($name in @('shuttli.exe','shuttli-windows.exe','shuttli-ui.exe')) {
    Copy-Item -LiteralPath (Join-Path $release $name) -Destination $stage
}
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'docs\install\windows.md') -Destination (Join-Path $stage 'README.md')
$files = foreach ($name in @('shuttli.exe','shuttli-windows.exe','shuttli-ui.exe','LICENSE','README.md')) {
    @{ name = $name; sha256 = (Get-FileHash -LiteralPath (Join-Path $stage $name) -Algorithm SHA256).Hash.ToLowerInvariant() }
}
@{ format = 1; version = $version; commit = $commit; target = 'x86_64-pc-windows-msvc'; files = @($files) } |
    ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stage 'build.json') -Encoding utf8
@'
@echo off
start "" "%~dp0shuttli-ui.exe"
'@ | Set-Content -LiteralPath (Join-Path $stage 'Open Shuttli.cmd') -Encoding ascii
$archive = Join-Path $OutputDirectory 'Shuttli-windows-x64.zip'
Compress-Archive -LiteralPath $stage -DestinationPath $archive -Force
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  Shuttli-windows-x64.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
Write-Output $archive
