param(
    [Parameter(Mandatory=$true)][string]$BinaryDirectory,
    [Parameter(Mandatory=$true)][string]$ProfileDirectory
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $ProfileDirectory) { throw 'Use a new, external test profile directory' }
$binary = Join-Path $BinaryDirectory 'shuttli.exe'
$native = Join-Path $BinaryDirectory 'shuttli-windows.exe'
$peer = Join-Path $BinaryDirectory 'examples\windows_acceptance.exe'
$priorProfile = $env:SHUTTLI_DATA_DIR
$agent = $null
function Api([string[]]$Arguments) {
    for ($retry=0; $retry -lt 20; $retry++) {
        $output = & $binary @Arguments --json
        $answer = $output | ConvertFrom-Json
        if ($LASTEXITCODE -eq 0) { return $answer }
        if ($answer.message -notlike 'operation in progress*') { throw "CLI failed: $output" }
        Start-Sleep -Milliseconds 100
    }
    throw "Agent remained busy: $output"
}
try {
    $env:SHUTTLI_DATA_DIR = $ProfileDirectory
    & $native self-test
    if ($LASTEXITCODE -ne 0) { throw 'Native clipboard acceptance failed' }
    $agent = Start-Process -FilePath $binary -ArgumentList daemon -WindowStyle Hidden -PassThru
    $ready = $false
    for ($attempt=0; $attempt -lt 20; $attempt++) {
        try { $status = Api -Arguments @('status'); $ready = $true; break } catch { Start-Sleep -Milliseconds 250 }
    }
    if (!$ready -or !$status.status.clipboard_available) { throw 'Windows clipboard did not become available' }
    $fingerprint = $status.status.device
    & $peer
    if ($LASTEXITCODE -ne 0) { throw 'Bidirectional TLS/clipboard acceptance failed' }
    $rows = (Api -Arguments @('history')).entries
    $text = $rows | Where-Object { $_.direction -eq 'receive' -and $_.format -eq 'text' } | Select-Object -First 1
    $expected = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String((Api -Arguments @('history','preview',"$($text.id)")).base64))
    $null = Api -Arguments @('history','copy',"$($text.id)",'--local-only')
    $actual = Get-Clipboard -Raw
    if ($actual.Replace("`r`n","`n") -cne $expected.Replace("`r`n","`n")) { throw 'History copy differs from independent Windows clipboard read' }
    Write-Output 'PASS: retained history copy and independent PowerShell clipboard read'
    $duplicate = Start-Process -FilePath $binary -ArgumentList daemon -WindowStyle Hidden -PassThru
    if (!$duplicate.WaitForExit(8000) -or $duplicate.ExitCode -eq 0) { throw 'Singleton lock failed' }
    Write-Output 'PASS: duplicate agent rejected'
    $null = Api -Arguments @('set','receive','off')
    $null = Api -Arguments @('quit')
    if (!$agent.WaitForExit(12000) -or $agent.ExitCode -ne 0) { throw 'Graceful quit failed' }
    $agent = Start-Process -FilePath $binary -ArgumentList daemon -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 1200
    $status = Api -Arguments @('status')
    if ($status.status.device -ne $fingerprint -or $status.status.settings.receive) { throw 'Restart did not preserve identity and settings' }
    Write-Output 'PASS: graceful quit/restart preserves identity and settings'
    $null = Api -Arguments @('quit')
    if (!$agent.WaitForExit(12000)) { throw 'Final quit failed' }
} finally {
    if ($agent -and !$agent.HasExited) { Stop-Process -Id $agent.Id }
    $env:SHUTTLI_DATA_DIR = $priorProfile
}
