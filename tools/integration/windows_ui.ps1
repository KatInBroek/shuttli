param(
    [Parameter(Mandatory=$true)][string]$BinaryDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [Parameter(Mandatory=$true)][string]$PythonExecutable
)
$ErrorActionPreference = 'Stop'
$generator = Join-Path $PSScriptRoot 'windows_ui_fixture.py'
$helper = Join-Path $BinaryDirectory 'shuttli-windows.exe'
$scenarios = @(
    @('light','en','light',850,620,1,1),
    @('dark','en','dark',850,620,1,1),
    @('german-min','de','light',720,520,1,1),
    @('french-2x','fr','dark',850,620,2,1),
    @('dutch','nl','light',850,620,1,1),
    @('text-200','de','light',720,520,1,2)
)
foreach ($scenario in $scenarios) {
    $folder = Join-Path $OutputDirectory $scenario[0]
    $fixture = Join-Path $folder 'fixture.json'
    & $PythonExecutable $generator --output $fixture --language $scenario[1] --theme $scenario[2] --text-scale $scenario[6]
    if ($LASTEXITCODE -ne 0) { throw "Fixture generation failed: $($scenario[0])" }
    & $helper ui-render $fixture (Join-Path $folder 'render') $scenario[5] $scenario[3] $scenario[4] *> (Join-Path $folder 'render.log')
    if ($LASTEXITCODE -ne 0) { throw "Visual acceptance failed: $($scenario[0]); inspect render.log" }
    Write-Output "PASS: $($scenario[0])"
}
