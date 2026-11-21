param(
    [string]$Version = "latest",
    [string]$InstallDir = "$HOME\.rystemdb\bin"
)
$ErrorActionPreference = "Stop"
$repo = "KeyserDSoze/RystemDatabase"
if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @("AMD64", "ARM64")) {
    throw "Only 64-bit Windows is supported."
}
if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
    throw "Native Windows ARM64 is not published yet."
}
if ($Version -eq "latest") {
    $releases = @(Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases?per_page=1")
    if ($releases.Count -eq 0) { throw "No published GitHub releases" }
    $Version = $releases[0].tag_name
}
if ($Version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$') { throw "Invalid release version" }
$asset = "rystemdb-windows-x64.exe"
$base = "https://github.com/$repo/releases/download/$Version"
$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("rystemdb-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
try {
    $binary = Join-Path $tempDir $asset
    $checksumFile = Join-Path $tempDir "SHA256SUMS"
    Invoke-WebRequest -Uri "$base/$asset" -OutFile $binary
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $checksumFile
    $checksumLine = Get-Content $checksumFile | Where-Object { $_ -match ('^[a-fA-F0-9]{64}\s+\*?' + [regex]::Escape($asset) + '$') } | Select-Object -First 1
    if (-not $checksumLine) { throw "Missing release checksum" }
    $expected = ($checksumLine -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Path $binary -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "SHA-256 mismatch" }
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item $binary (Join-Path $InstallDir "rystemdb.exe") -Force
    $path = [Environment]::GetEnvironmentVariable("Path", "User")
    if (($path -split ';') -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable("Path", (($path.TrimEnd(';') + ";" + $InstallDir).TrimStart(';')), "User")
    }
    Write-Host "Installed Rystem Database $Version. Open a new terminal and run rystemdb --version"
} finally {
    Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}
