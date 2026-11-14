# Hosted Windows only. Never targets an existing worker or a shared browser.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$engine = Join-Path $repo 'refact-agent/engine'
$evidence = Join-Path $env:RUNNER_TEMP 'browser-windows-roadmap-evidence'
New-Item -ItemType Directory -Force $evidence | Out-Null
$owned = Join-Path $env:RUNNER_TEMP ('browser-roadmap-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $owned | Out-Null
$worker = $null
$roadmap = $null
$failed = $false
$failures = [System.Collections.Generic.List[string]]::new()
$observed = @{}
$inspectionFailed = $false
$controlRoot = $null
function Capture-OwnedTree {
    # PID plus creation time, not PID alone, is the process identity. Only follow
    # parent links while the parent identity is still present in the same snapshot.
    $all = @(Get-CimInstance Win32_Process -ErrorAction Stop)
    $live = @{}
    foreach ($p in $all) { $live[[int]$p.ProcessId] = $p }
    $root = $live[[int]$worker.Id]
    if (-not $root -or -not $root.CreationDate) { throw 'Cannot inspect live worker identity' }
    $known = @{}; $known[[int]$worker.Id] = $true
    do {
        $added = $false
        foreach ($p in $all) {
            $id = [int]$p.ProcessId
            if (-not $known.ContainsKey($id) -and $known.ContainsKey([int]$p.ParentProcessId)) {
                if (-not $p.CreationDate -or -not $p.Name -or -not $p.CommandLine) { throw "Cannot inspect owned descendant $id" }
                $known[$id] = $true; $added = $true
            }
        }
    } while ($added)
    foreach ($id in $known.Keys) {
        $p = $live[$id]
        $key = "$id/$($p.CreationDate.ToUniversalTime().ToString('o'))"
        $script:observed[$key] = [ordered]@{ pid = $id; parent = [int]$p.ParentProcessId; created = $p.CreationDate.ToUniversalTime().ToString('o'); name = $p.Name; command = $p.CommandLine }
    }
}
function Fail([string]$message) { $script:failed = $true; $script:failures.Add($message); Write-Warning $message }
function Write-KnownFolderEvidence([string]$phase) {
    $receipt = [ordered]@{ phase = $phase; chrome = [ordered]@{ path = $env:CHROME }; environment = [ordered]@{}; known_folders = [ordered]@{}; policies = @() }
    try {
        $receipt.chrome.file_version = (Get-Item -LiteralPath $env:CHROME).VersionInfo.FileVersion
        $receipt.chrome.sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $env:CHROME).Hash
    } catch { $receipt.chrome.error = "$_" }
    foreach ($name in @('USERPROFILE','LOCALAPPDATA','APPDATA','TEMP','TMP')) { $receipt.environment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    foreach ($folder in @('LocalApplicationData','ApplicationData','UserProfile')) {
        foreach ($option in @('None','DoNotVerify')) {
            try { $receipt.known_folders["$folder/$option"] = [Environment]::GetFolderPath([Environment+SpecialFolder]$folder, [Environment+SpecialFolderOption]$option) }
            catch { $receipt.known_folders["$folder/$option"] = [ordered]@{ error = "$_" } }
        }
    }
    foreach ($hive in @('LocalMachine','CurrentUser')) {
        foreach ($view in @('Registry64','Registry32')) {
            $base = $null; $key = $null
            $policy = [ordered]@{ hive = $hive; view = $view; path = 'SOFTWARE\Policies\Google\Chrome'; values = [ordered]@{} }
            try {
                $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]$hive, [Microsoft.Win32.RegistryView]$view)
                $key = $base.OpenSubKey($policy.path, $false)
                foreach ($name in @('UserDataDir','RemoteDebuggingAllowed')) {
                    $value = if ($key) { $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { $null }
                    $policy.values[$name] = [ordered]@{ present = ($null -ne $value); value = $value }
                }
            } catch { $policy.error = "$_" }
            finally { if ($key) { $key.Dispose() }; if ($base) { $base.Dispose() } }
            $receipt.policies += $policy
        }
    }
    $receipt | ConvertTo-Json -Depth 7 | Set-Content -Encoding utf8 (Join-Path $evidence ("known-folders-$phase.json"))
}
function Invoke-PlainChromeControl([string]$chromePath) {
    $root = Join-Path $env:RUNNER_TEMP ('browser-control-' + [guid]::NewGuid().ToString('N'))
    $script:controlRoot = $root
    $profileDir = Join-Path $root 'profile'
    New-Item -ItemType Directory -Force $profileDir | Out-Null
    $profileArg = '--user-data-dir="' + $profileDir + '"'
    $vendorDefaults = @('--disable-background-networking','--disable-background-timer-throttling','--disable-backgrounding-occluded-windows','--disable-breakpad','--disable-client-side-phishing-detection','--disable-component-extensions-with-background-pages','--disable-default-apps','--disable-dev-shm-usage','--disable-extensions','--disable-features=TranslateUI,BlinkGenPropertyTrees','--disable-hang-monitor','--disable-ipc-flooding-protection','--disable-popup-blocking','--disable-prompt-on-repost','--disable-renderer-backgrounding','--disable-sync','--force-color-profile=srgb','--metrics-recording-only','--no-first-run','--password-store=basic','--use-mock-keychain')
    $mandatory = @('--no-restore-last-session','--no-first-run','--no-startup-window','--disable-blink-features=AutomationControlled','--no-default-browser-check','--disable-search-engine-choice-screen','--disable-back-forward-cache','--log-level=2')
    $arguments = @('--remote-debugging-port=0','--verbose','--log-level=0','--no-first-run',$profileArg) + $vendorDefaults + $mandatory + @('--headless','--ignore-certificate-errors','--disable-gpu','--no-sandbox','--disable-setuid-sandbox')
    $stderr = Join-Path $evidence 'plain-control.stderr.log'
    $receipt = [ordered]@{ chrome = $chromePath; chrome_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $chromePath).Hash; root = $root; profile_argument = $profileArg; arguments = $arguments; pid = $null; created = $null; command = $null; devtools_ready = $false; kill_requested = $false; root_exit_confirmed = $false; exit_code = $null; descendants = @(); leftovers = @(); root_associated_alive = @(); errors = @() }
    $proc = $null
    try {
        $proc = Start-Process -FilePath $chromePath -ArgumentList $arguments -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $evidence 'plain-control.stdout.log') -RedirectStandardError $stderr
        $receipt.pid = $proc.Id
        $self = Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction Stop
        if ($self -and $self.CreationDate) { $receipt.created = $self.CreationDate.ToUniversalTime().ToString('o'); $receipt.command = $self.CommandLine }
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while (-not $proc.HasExited -and [DateTime]::UtcNow -lt $deadline) {
            if ((Test-Path -LiteralPath (Join-Path $profileDir 'DevToolsActivePort') -PathType Leaf) -or ((Test-Path -LiteralPath $stderr) -and (Select-String -LiteralPath $stderr -Pattern 'DevTools listening on ws://' -SimpleMatch -Quiet -ErrorAction SilentlyContinue))) { $receipt.devtools_ready = $true; break }
            Start-Sleep -Milliseconds 250
        }
        if ($receipt.created) {
            $all = @(Get-CimInstance Win32_Process -ErrorAction Stop)
            $known = @{ ([int]$proc.Id) = $self.CreationDate }
            do {
                $added = $false
                foreach ($p in $all) {
                    $id = [int]$p.ProcessId; $parent = [int]$p.ParentProcessId
                    if (-not $known.ContainsKey($id) -and $known.ContainsKey($parent) -and $p.CreationDate -and $p.CreationDate -ge $known[$parent]) {
                        $known[$id] = $p.CreationDate; $added = $true
                        $receipt.descendants += [ordered]@{ pid = $id; parent = $parent; created = $p.CreationDate.ToUniversalTime().ToString('o'); name = $p.Name; command = $p.CommandLine; profile_associated = [bool]($p.CommandLine -and $p.CommandLine.Contains($root)) }
                    }
                }
            } while ($added)
        }
    } catch { $receipt.errors += "probe: $_" }
    finally {
        if ($proc) {
            try {
                if (-not $proc.HasExited) { $receipt.kill_requested = $true; $proc.Kill() }
                $receipt.root_exit_confirmed = $proc.WaitForExit(10000)
                if ($receipt.root_exit_confirmed) { $receipt.exit_code = $proc.ExitCode }
            } catch { $receipt.errors += "root cleanup: $_" }
        }
        try {
            Start-Sleep -Seconds 2
            foreach ($d in $receipt.descendants) {
                $p = Get-CimInstance Win32_Process -Filter "ProcessId = $($d.pid)" -ErrorAction Stop
                if ($p -and $p.CreationDate -and $p.CreationDate.ToUniversalTime().ToString('o') -eq $d.created) { $receipt.leftovers += $d }
            }
            $receipt.root_associated_alive = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($root) } | ForEach-Object { [ordered]@{ pid = [int]$_.ProcessId; parent = [int]$_.ParentProcessId; name = $_.Name; command = $_.CommandLine } })
        } catch { $receipt.errors += "leftover inspection: $_" }
        if ($receipt.root_exit_confirmed -and $receipt.leftovers.Count -eq 0 -and $receipt.root_associated_alive.Count -eq 0) {
            try { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop; $receipt.root_removed = $true } catch { $receipt.errors += "root removal: $_" }
        } else { $receipt.root_removed = $false }
        $receipt | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 (Join-Path $evidence 'plain-control.json')
    }
    $problems = @()
    if (-not $receipt.devtools_ready) { $problems += 'no DevTools endpoint within 30s' }
    if (-not $receipt.root_exit_confirmed) { $problems += 'root exit unconfirmed' }
    if ($receipt.leftovers.Count -or $receipt.root_associated_alive.Count) { $problems += 'control processes still alive; none killed beyond the receipt root' }
    if ($receipt.errors.Count) { $problems += $receipt.errors }
    return ($problems -join '; ')
}
function Run-Logged([string]$name, [string]$exe, [string[]]$arguments, [bool]$requireTests = $false) {
    $log = Join-Path $evidence ($name + '.log')
    Write-Host "Running $name"
    & $exe @arguments 2>&1 | Out-File -FilePath $log -Encoding utf8
    $code = $LASTEXITCODE
    if ($code -ne 0) { throw "$name exited $code; see $log" }
    if ($requireTests) {
        $text = Get-Content -Raw $log
        if ($text -notmatch 'test result: ok\. [1-9][0-9]* passed; 0 failed; 0 ignored;') { throw "$name did not execute non-ignored tests; see $log" }
    }
}
try {
    Set-Location $repo
    $head = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $env:BROWSER_QUALIFICATION_SHA -cnotmatch '^[0-9a-f]{40}$' -or $head -cne $env:BROWSER_QUALIFICATION_SHA) { throw 'Exact full HEAD SHA is required' }
    # Tracked source must be pristine; the workflow removes only local cargo defaults.
    $dirty = @(git status --porcelain --untracked-files=no -- . ':!.cargo/config.toml')
    if ($dirty.Count -ne 0) { throw 'Tracked source is modified' }
    $pins = @('HEAD ' + $head)
    $files = @(git ls-files -- refact-agent/engine tools/dev/browser_windows_roadmap_qualification.ps1 .github/workflows/agent_engine_check.yml)
    foreach ($file in $files) {
        if (Test-Path -LiteralPath $file -PathType Leaf) { $pins += ((Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash + '  ' + $file) }
    }
    $pins | Set-Content -Encoding utf8 (Join-Path $evidence 'source-pins.txt')
    $chrome = @("${env:ProgramFiles}\Google\Chrome\Application\chrome.exe", "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $chrome) { throw 'Hosted Chrome executable not found' }
    $env:CHROME = $chrome
    Write-KnownFolderEvidence 'before-isolation'
    $env:LIBCLANG_PATH = Join-Path $env:ProgramFiles 'LLVM/bin'
    if (-not (Test-Path (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) { throw 'LLVM libclang.dll missing' }
    $env:RUST_MIN_STACK = '16777216'
    $env:REFACT_BROWSER_E2E = '1'
    Set-Location $engine
    Run-Logged 'precompile-root-lib-tests' 'cargo' @('test','--locked','--profile','ci-test','--lib','--no-run')
    Run-Logged 'precompile-engine' 'cargo' @('build','--locked','--profile','ci-test','--bin','refact-lsp')
    # Isolate tests as well as worker; do not modify the hosted runner's real profile.
    if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $env:USERPROFILE '.rustup' }
    if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $env:USERPROFILE '.cargo' }
    $env:TEMP = $owned; $env:TMP = $owned
    $env:HOME = Join-Path $owned 'home'; $env:USERPROFILE = $env:HOME
    $env:LOCALAPPDATA = Join-Path $env:USERPROFILE 'AppData/Local'; $env:APPDATA = Join-Path $env:USERPROFILE 'AppData/Roaming'
    $env:XDG_CACHE_HOME = Join-Path $owned 'cache'; $env:XDG_CONFIG_HOME = Join-Path $owned 'config'
    foreach ($dir in @($env:HOME,$env:LOCALAPPDATA,$env:APPDATA,$env:XDG_CACHE_HOME,$env:XDG_CONFIG_HOME)) { New-Item -ItemType Directory -Force $dir | Out-Null }
    Write-KnownFolderEvidence 'after-isolation'
    Set-Location $engine
    Run-Logged 'python-version' 'python' @('--version')
    Run-Logged 'windows-profiles' 'cargo' @('test','--locked','--profile','ci-test','-p','refact-browser','--lib','windows_profiles::tests','--','--nocapture','--test-threads=1') $true
    Run-Logged 'vendor-transport' 'cargo' @('test','--locked','--profile','ci-test','-p','headless_chrome','--lib','transport::','--','--nocapture','--test-threads=1') $true
    Run-Logged 'vendor-launch-tracker' 'cargo' @('test','--locked','--profile','ci-test','-p','headless_chrome','--lib','launch_tracker','--','--nocapture','--test-threads=1') $true
    Run-Logged 'vendor-owned-process' 'cargo' @('test','--locked','--profile','ci-test','-p','headless_chrome','--lib','owned_process_','--','--nocapture','--test-threads=1') $true
    Run-Logged 'vendor-startup-deadline' 'cargo' @('test','--locked','--profile','ci-test','-p','headless_chrome','--lib','pending_stderr_obeys_original_startup_deadline','--','--nocapture','--test-threads=1') $true
    'Launch-tracker spawned-child fault fixtures are Unix-only; Windows runs the missing-executable tracker test, owned-process tests, startup deadline test and real Chrome lifecycle.' | Set-Content -Encoding utf8 (Join-Path $evidence 'coverage-limitations.txt')
    try { $controlFailure = Invoke-PlainChromeControl $chrome } catch { $controlFailure = "$_" }
    if ($controlFailure) { Fail "Plain Chrome control: $controlFailure" }
    Run-Logged 'named-sessions' 'cargo' @('test','--locked','--profile','ci-test','--lib','integrations::browser_runtime::named_sessions::tests','--','--include-ignored','--nocapture','--test-threads=1') $true
    Run-Logged 'named-live-chrome' 'cargo' @('test','--locked','--profile','ci-test','--lib','live_named_children_are_isolated_and_generation_fenced','--','--include-ignored','--nocapture','--test-threads=1') $true
    Run-Logged 'tool-chrome' 'cargo' @('test','--locked','--profile','ci-test','--lib','tools::tool_chrome::tests','--','--include-ignored','--nocapture','--test-threads=1') $true
    Run-Logged 'build-engine' 'cargo' @('build','--locked','--profile','ci-test','--bin','refact-lsp')
    $binary = (Resolve-Path 'target/ci-test/refact-lsp.exe').Path
    $hash = (Get-FileHash -Algorithm SHA256 $binary).Hash
    "binary_sha256 $hash`nchrome $chrome`nchrome_sha256 $((Get-FileHash -Algorithm SHA256 $chrome).Hash)" | Set-Content -Encoding utf8 (Join-Path $evidence 'runtime-pins.txt')
    if (Get-NetTCPConnection -LocalPort 18991 -State Listen -ErrorAction SilentlyContinue) { throw 'Port 18991 already occupied; refusing to contact it' }
    $project = Join-Path $owned 'project'
    New-Item -ItemType Directory $project | Out-Null
    $worker = Start-Process -FilePath $binary -ArgumentList @('--http-port','18991','--http-host','127.0.0.1','--lsp-port','0','--experimental','--no-scheduler','--logs-stderr',('--workspace-folder="' + $project + '"')) -PassThru -RedirectStandardOutput (Join-Path $evidence 'engine.stdout.log') -RedirectStandardError (Join-Path $evidence 'engine.stderr.log')
    "pid $($worker.Id)`nstart_time_utc $($worker.StartTime.ToUniversalTime().ToString('o'))" | Add-Content -Encoding utf8 (Join-Path $evidence 'runtime-pins.txt')
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        if ($worker.HasExited) { throw 'Worker exited before acquiring its listener' }
        $listener = Get-NetTCPConnection -LocalPort 18991 -State Listen -ErrorAction SilentlyContinue
        if ($listener -and $listener.OwningProcess -ne $worker.Id) { throw 'Foreign listener appeared; refusing to contact it' }
        if (-not $listener) { Start-Sleep -Milliseconds 250 }
    } until ($listener -or [DateTime]::UtcNow -gt $deadline)
    if (-not $listener) { throw 'Worker listener did not appear' }
    $tools = Invoke-RestMethod -Uri 'http://127.0.0.1:18991/v1/tools' -TimeoutSec 30
    $chromeTool = @($tools | ForEach-Object { $_.tools } | Where-Object { $_.spec.name -eq 'chrome' })
    if ($chromeTool.Count -ne 1 -or -not $chromeTool[0].enabled) { throw 'Isolated Chrome tool is not uniquely enabled' }
    $schema = $chromeTool[0].spec | ConvertTo-Json -Depth 100 -Compress
    foreach ($feature in @('create_session','expected_session_id','search_snapshot','network_requests')) { if (-not $schema.Contains($feature)) { throw "Chrome schema lacks $feature" } }
    $schema | Set-Content -Encoding utf8 (Join-Path $evidence 'chrome-schema.json')
    # Use argument quoting because runner.temp can contain spaces.
    $python = (Get-Command python -CommandType Application | Select-Object -First 1).Source
    $receiptDir = Join-Path $evidence 'roadmap'
    $roadmap = Start-Process -FilePath $python -ArgumentList @('tests/test_native_browser_roadmap.py','--url','http://127.0.0.1:18991','--output-dir',('"' + $receiptDir + '"'),'--eviction') -PassThru -RedirectStandardOutput (Join-Path $evidence 'roadmap.stdout.log') -RedirectStandardError (Join-Path $evidence 'roadmap.stderr.log')
    while (-not $roadmap.WaitForExit(250)) {
        if ($worker.HasExited) { throw 'Worker exited during roadmap' }
        try { Capture-OwnedTree } catch { $script:inspectionFailed = $true; throw }
    }
    if ($roadmap.ExitCode -ne 0) { throw "Roadmap exited $($roadmap.ExitCode)" }
    if (@($observed.Values | Where-Object { $_.name -ieq 'chrome.exe' }).Count -eq 0) { throw 'No owned Chrome process was observed during roadmap' }
    $summary = Get-Content -Raw (Join-Path $evidence 'roadmap/summary.json') | ConvertFrom-Json
    if (-not $summary.passed -or @($summary.cases.PSObject.Properties).Count -ne 6 -or @($summary.cases.PSObject.Properties | Where-Object { -not $_.Value.passed }).Count -ne 0) { throw 'Roadmap did not produce six passing cases' }
    if (@($summary.not_covered).Count -ne 0) { throw 'Roadmap reports uncovered scenarios' }
} catch { Fail $_.ToString() }
finally {
    if ($roadmap -and -not $roadmap.HasExited) {
        Fail 'Roadmap harness remained running during cleanup'
        $roadmap.Kill(); $roadmap.WaitForExit()
    }
    if ($worker -and -not $worker.HasExited) {
        try { Capture-OwnedTree } catch { $inspectionFailed = $true; Fail "Pre-shutdown process inspection: $_" }
    }
    if ($worker) {
        try {
            if (-not $worker.HasExited) {
                $listener = Get-NetTCPConnection -LocalPort 18991 -State Listen -ErrorAction SilentlyContinue
                if ($listener.OwningProcess -eq $worker.Id) { Invoke-WebRequest -Method Post -Uri 'http://127.0.0.1:18991/v1/graceful-shutdown' -TimeoutSec 10 | Out-Null }
                else { Fail 'Worker does not own expected listener; refusing HTTP shutdown' }
                if (-not $worker.WaitForExit(30000)) { Fail 'Worker did not shut down gracefully'; $worker.Kill(); $worker.WaitForExit() }
            }
        } catch { Fail "Worker cleanup: $_"; if (-not $worker.HasExited) { $worker.Kill(); $worker.WaitForExit() } }
    }
    # Never kill an unknown PID. Report even orphaned profile-associated Chrome as failure.
    try {
        $observed.Values | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $evidence 'observed-processes.json')
        if ($inspectionFailed) { Fail 'Owned process inspection was incomplete' }
        $remaining = @()
        foreach ($entry in $observed.Values) {
            $p = Get-CimInstance Win32_Process -Filter "ProcessId = $($entry.pid)" -ErrorAction Stop
            if ($p -and -not $p.CreationDate) { throw "Cannot inspect process identity $($entry.pid)" }
            if ($p -and $p.CreationDate.ToUniversalTime().ToString('o') -eq $entry.created) { $remaining += $entry }
        }
        $remaining | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $evidence 'observed-leftovers.json')
        if ($remaining.Count) { Fail "Found $($remaining.Count) observed owned processes still alive" }
        $left = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'chrome.exe' -and $_.CommandLine -and $_.CommandLine.Contains($owned) })
        $left | Select-Object ProcessId,ParentProcessId,CreationDate,Name,CommandLine | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $evidence 'chrome-leftovers.json')
        if ($left.Count) { Fail "Found $($left.Count) profile-associated Chrome processes after shutdown; no unknown process was killed" }
        $profiles = @(Get-ChildItem -Path $owned -Directory -Recurse -ErrorAction Stop | Where-Object { $_.Name -eq 'named_browser_profiles' } | ForEach-Object { Get-ChildItem -Force $_.FullName -ErrorAction Stop } | Where-Object { -not ($_.Name -eq '.allocation.lock' -and -not $_.PSIsContainer -and -not ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) })
        if ($profiles.Count) { Fail "Found $($profiles.Count) named profile leftovers" }
    } catch { Fail "Cleanup inspection: $_" }
    @{ passed = -not $failed; failures = @($failures); owned_root = $owned; control_root = $controlRoot } | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 (Join-Path $evidence 'qualification-result.json')
}
if ($failed) { exit 1 }
