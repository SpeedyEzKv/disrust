[CmdletBinding()]
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\noir-chat.exe'),
    [ValidateRange(1, 10)][int]$Runs = 3,
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Exe).Path
$runRoot = Join-Path ([IO.Path]::GetTempPath()) ('noir-chat-check-' + [guid]::NewGuid().ToString('N'))
New-Item -Path $runRoot -ItemType Directory | Out-Null

function Read-Probe([string]$Path) {
    $probe = @{}
    foreach ($line in (Get-Content -LiteralPath $Path -Encoding UTF8)) {
        $key, $value = $line -split '=', 2
        $probe[$key] = $value
    }
    return $probe
}

function Snapshot([int]$AppPid, [int]$BrowserPid) {
    $pids = [Collections.Generic.HashSet[int]]::new()
    $null = $pids.Add($AppPid)
    $null = $pids.Add($BrowserPid)
    $processTree = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    do {
        $previousCount = $pids.Count
        foreach ($entry in $processTree) {
            if ($pids.Contains($entry.ParentProcessId)) {
                $null = $pids.Add($entry.ProcessId)
            }
        }
    } while ($previousCount -ne $pids.Count)

    $sample = @{}
    foreach ($p in (Get-Process -Id @($pids) -ErrorAction SilentlyContinue)) {
        $sample[$p.Id] = @{
            CpuSeconds = $p.TotalProcessorTime.TotalSeconds
            PrivateBytes = $p.PrivateMemorySize64
        }
    }
    return $sample
}

try {
    $results = foreach ($run in 1..$Runs) {
        $probePath = Join-Path $runRoot ('run-' + $run)
        New-Item -Path $probePath -ItemType Directory | Out-Null
        $argLine = '--diagnose "' + $probePath + '"'
        if ($Offline) { $argLine += ' --offline' }
        $p = Start-Process -FilePath $exePath -ArgumentList $argLine -WindowStyle Hidden -PassThru
        try {
            $navigationPath = Join-Path $probePath 'navigation.txt'
            $latencyPath = Join-Path $probePath 'ui-latency.txt'
            $browserPath = Join-Path $probePath 'browser-check.json'
            $until = [datetime]::UtcNow.AddSeconds(49)
            while ((!(Test-Path -LiteralPath $browserPath) -or !(Test-Path -LiteralPath $latencyPath)) -and
                [datetime]::UtcNow -lt $until -and !$p.HasExited) {
                Start-Sleep -Milliseconds 100
            }
            $errorPath = Join-Path $probePath 'startup-error.txt'
            if (Test-Path -LiteralPath $errorPath) {
                throw "WebView2 startup: $(Get-Content -LiteralPath $errorPath -Raw -Encoding UTF8)"
            }
            if (!(Test-Path -LiteralPath $browserPath) -or !(Test-Path -LiteralPath $latencyPath)) {
                throw "Measurement timed out in run $run."
            }
            $nav = Read-Probe $navigationPath
            $latency = Read-Probe $latencyPath
            $browser = Get-Content -LiteralPath $browserPath -Raw -Encoding UTF8 | ConvertFrom-Json
            $networkPath = Join-Path $probePath 'network-errors.txt'
            $networkErrors = if (Test-Path -LiteralPath $networkPath) {
                @(Get-Content -LiteralPath $networkPath | Group-Object | Select-Object Name,Count)
            }

            $before = Snapshot $p.Id ([int]$nav.pid)
            $timer = [Diagnostics.Stopwatch]::StartNew()
            Start-Sleep -Seconds 5
            $after = Snapshot $p.Id ([int]$nav.pid)
            $timer.Stop()
            if (!$after.ContainsKey($p.Id) -or !$after.ContainsKey([int]$nav.pid)) {
                throw "The host or browser process ended during run $run."
            }
            $cpuSeconds = 0.0
            foreach ($childId in $after.Keys) {
                if ($before.ContainsKey($childId)) {
                    $cpuSeconds += [math]::Max(0, $after[$childId].CpuSeconds - $before[$childId].CpuSeconds)
                }
            }
            $totalPrivate = ($after.Values | ForEach-Object { $_.PrivateBytes } | Measure-Object -Sum).Sum
            [pscustomobject]@{
                run = $run
                scenario = if ($Offline) { 'offline' } else { 'discord-signed-out' }
                hostReadyMs = [int](Get-Content -LiteralPath (Join-Path $probePath 'host-ready.txt'))
                navigationMs = [int]$nav.elapsed_ms
                pageReadyMs = if (Test-Path -LiteralPath (Join-Path $probePath 'page-ready.txt')) {
                    [int](Get-Content -LiteralPath (Join-Path $probePath 'page-ready.txt'))
                }
                startupRetries = if (Test-Path -LiteralPath (Join-Path $probePath 'startup-retry.txt')) {
                    [int](Get-Content -LiteralPath (Join-Path $probePath 'startup-retry.txt'))
                } else { 0 }
                navigationSuccess = $nav.success -eq 'true'
                httpStatus = [int]$nav.http_status
                title = $nav.title
                browser = $browser
                networkErrors = $networkErrors
                processCount = $after.Count
                hostPrivateMiB = [math]::Round($after[$p.Id].PrivateBytes / 1MB, 2)
                treePrivateMiB = [math]::Round($totalPrivate / 1MB, 2)
                idleCpuPercentOfOneCore = [math]::Round(100 * $cpuSeconds / $timer.Elapsed.TotalSeconds, 2)
                mainLoopP95Us = [int]$latency.p95_us
                mainLoopMaxUs = [int]$latency.max_us
            }
        } finally {
            if (!$p.HasExited) {
                New-Item -Path (Join-Path $probePath 'stop') -ItemType File -Force | Out-Null
                if (!$p.WaitForExit(6000)) { Stop-Process -InputObject $p }
            }
        }
    }
    ConvertTo-Json -InputObject @($results) -Depth 4
    $failed = $results | Where-Object {
        !$_.navigationSuccess -or
        ($Offline -and !$_.browser.offlinePage) -or
        (!$Offline -and ($_.httpStatus -ne 200 -or !$_.browser.discordOrigin -or !$_.browser.loginForm))
    }
    if (@($failed).Count -gt 0) {
        throw 'WebView2 did not render the expected page. See the reported navigation and DOM status.'
    }
} finally {
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $resolved = (Resolve-Path -LiteralPath $runRoot).Path
    if ($resolved.StartsWith($temp + 'noir-chat-check-', [StringComparison]::OrdinalIgnoreCase)) {
        # WebView2 children may still be flushing the isolated profile.
        for ($attempt = 0; $attempt -lt 8; $attempt++) {
            try {
                Remove-Item -LiteralPath $resolved -Recurse -Force
                break
            } catch {
                if ($attempt -eq 7) { Write-Warning "Temporary diagnostic profile remains at $resolved" }
                Start-Sleep -Milliseconds 250
            }
        }
    }
}
