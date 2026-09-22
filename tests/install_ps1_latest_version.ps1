Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$installPs1 = Join-Path (Split-Path -Parent $PSScriptRoot) "install.ps1"
if (-not (Test-Path -LiteralPath $installPs1 -PathType Leaf)) {
    [Console]::Error.WriteLine("error: install.ps1 not found at $installPs1")
    exit 1
}

$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($installPs1, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) {
    [Console]::Error.WriteLine("error: install.ps1 does not parse: $($parseErrors[0].Message)")
    exit 1
}

$definitions = $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)
foreach ($name in @("Normalize-Version", "Get-LatestVersion")) {
    $definition = $definitions | Where-Object { $_.Name -eq $name } | Select-Object -First 1
    if ($null -eq $definition) {
        [Console]::Error.WriteLine("error: install.ps1 does not define $name")
        exit 1
    }
    . ([scriptblock]::Create($definition.Extent.Text))
}

$ApiUrl = "https://api.github.com/repos/JegernOUTT/refact"

function Fail([string]$Message) {
    throw $Message
}

$script:responseShape = "enumerated"
$script:fixture = @()

function Invoke-RestMethod {
    param([string]$Uri, [hashtable]$Headers)
    if ($script:responseShape -eq "single-array") {
        return ,$script:fixture
    }
    return $script:fixture
}

function New-Releases([string[]]$Tags) {
    return @($Tags | ForEach-Object { [pscustomobject]@{ tag_name = $_ } })
}

$script:failures = 0

function Write-TestPass([string]$Label) {
    Write-Host "ok: $Label"
}

function Write-TestFailure([string]$Label) {
    [Console]::Error.WriteLine("FAIL: $Label")
    $script:failures++
}

$mixedTags = @(
    "release/v8.6.4",
    "engine/v8.6.4",
    "release/v8.6.3",
    "engine/v8.6.3",
    "engine/v8.6.3-main-415-cbfca9ec",
    "release/v8.6.2"
)

foreach ($shape in @("enumerated", "single-array")) {
    $script:responseShape = $shape

    $script:fixture = New-Releases $mixedTags
    $resolved = Get-LatestVersion
    if ($resolved -eq "8.6.4") {
        Write-TestPass "$shape response resolves the newest engine release"
    } else {
        Write-TestFailure "$shape response resolved '$resolved' instead of '8.6.4'"
    }

    $script:fixture = New-Releases @("release/v8.6.4", "release/v8.6.3")
    try {
        $resolved = Get-LatestVersion
        Write-TestFailure "$shape response without engine releases resolved '$resolved' instead of failing"
    } catch {
        if ($_.Exception.Message -like "could not find an engine/v* release in *") {
            Write-TestPass "$shape response without engine releases fails clearly"
        } else {
            Write-TestFailure "$shape response without engine releases failed unexpectedly: $($_.Exception.Message)"
        }
    }
}

$normalizeCases = [ordered]@{
    "engine/v8.6.4" = "8.6.4"
    "engine/8.6.4" = "8.6.4"
    "release/v8.6.4" = "8.6.4"
    "v8.6.4" = "8.6.4"
    "8.6.4" = "8.6.4"
}
foreach ($case in $normalizeCases.GetEnumerator()) {
    $normalized = Normalize-Version $case.Key
    if ($normalized -eq $case.Value) {
        Write-TestPass "Normalize-Version '$($case.Key)' -> '$($case.Value)'"
    } else {
        Write-TestFailure "Normalize-Version '$($case.Key)' returned '$normalized' instead of '$($case.Value)'"
    }
}

if ($script:failures -ne 0) {
    [Console]::Error.WriteLine("")
    [Console]::Error.WriteLine("$($script:failures) test(s) failed")
    exit 1
}

Write-Host ""
Write-Host "All install.ps1 latest version tests passed"
