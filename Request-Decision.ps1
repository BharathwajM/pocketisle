param(
    [string]$RequestPath,
    [string]$Question,
    [string]$Detail = '',
    [string[]]$Options = @('Yes', 'No'),
    [string]$Session = '',
    [ValidateRange(10, 3600)][int]$TimeoutSeconds = 3600
)
$ErrorActionPreference = 'Stop'
$islandExecutable = Join-Path $env:LOCALAPPDATA 'PocketIsle\pocketisle.exe'
if (-not (Test-Path -LiteralPath $islandExecutable)) { throw 'Pocket Isle is not installed.' }
if ($RequestPath) {
    $request = Get-Content -LiteralPath $RequestPath -Raw | ConvertFrom-Json
} else {
    if (-not $Question) { throw 'Provide -Question or -RequestPath.' }
    $choices = for ($i = 0; $i -lt $Options.Count; $i++) {
        @{ id = "option-$($i + 1)"; label = $Options[$i] }
    }
    $request = @{ question = $Question; detail = $Detail; options = @($choices); timeout_seconds = $TimeoutSeconds }
    if ($Session) { $request.session = $Session }
}
if (-not (Get-Process -Name pocketisle -ErrorAction SilentlyContinue)) {
    Start-Process -FilePath $islandExecutable -WindowStyle Hidden
    Start-Sleep -Seconds 2
}
$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $islandExecutable
$startInfo.ArgumentList.Add('ask')
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.StandardInputEncoding = [System.Text.UTF8Encoding]::new($false)
$startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
try {
    if (-not $process.Start()) { throw 'Cannot start the island request.' }
    $outputTask = $process.StandardOutput.ReadToEndAsync()
    $errorTask = $process.StandardError.ReadToEndAsync()
    $process.StandardInput.Write(($request | ConvertTo-Json -Depth 12 -Compress))
    $process.StandardInput.Close()
    $process.WaitForExit()
    $resultText = $outputTask.GetAwaiter().GetResult().Trim()
    $errorText = $errorTask.GetAwaiter().GetResult().Trim()
    if ($process.ExitCode -ne 0 -or -not $resultText) {
        throw "The island request failed. Ask in the chat instead. $errorText"
    }
    $null = $resultText | ConvertFrom-Json -ErrorAction Stop
    Write-Output $resultText
} finally {
    $process.Dispose()
}
