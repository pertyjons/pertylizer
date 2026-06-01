# Assemble a Pertylizer release package directory and zip it (Windows).
#
# Usage: pwsh packaging/assemble.ps1 -Os windows -Arch x86_64
#
# Expects release binaries already built:
#   target\release\pertylizer.exe
#   visualizer\target\release\pertylizer-visualizer.exe
param(
    [string]$Os = "windows",
    [string]$Arch = "x86_64"
)
$ErrorActionPreference = "Stop"

$version = (Get-Content crates\pertylizer\Cargo.toml |
    Where-Object { $_ -match '^version\s*=' } |
    Select-Object -First 1) -replace '.*"(.*)".*', '$1'
$pkg = "pertylizer-$version-$Os-$Arch"

if (Test-Path $pkg) { Remove-Item -Recurse -Force $pkg }
if (Test-Path "$pkg.zip") { Remove-Item -Force "$pkg.zip" }
New-Item -ItemType Directory -Force -Path $pkg | Out-Null

# Binaries
Copy-Item target\release\pertylizer.exe (Join-Path $pkg "pertylizer.exe")
Copy-Item visualizer\target\release\pertylizer-visualizer.exe (Join-Path $pkg "pertylizer-visualizer.exe")

# Examples
Copy-Item -Recurse assets\examples (Join-Path $pkg "examples")

# Config + docs
Copy-Item packaging\README.md (Join-Path $pkg "README.md")
Copy-Item packaging\pertylizer.toml (Join-Path $pkg "pertylizer.toml")

# MCP configs (folder + project-local convenience copies)
Copy-Item -Recurse packaging\mcp (Join-Path $pkg "mcp")
Copy-Item packaging\mcp\claude-code\.mcp.json (Join-Path $pkg ".mcp.json")
New-Item -ItemType Directory -Force -Path (Join-Path $pkg ".gemini") | Out-Null
Copy-Item packaging\mcp\gemini\settings.json (Join-Path $pkg ".gemini\settings.json")

# .ai scaffold
New-Item -ItemType Directory -Force -Path (Join-Path $pkg ".ai") | Out-Null
Copy-Item packaging\ai-scaffold\README.md (Join-Path $pkg ".ai\README.md")

# Zip
Compress-Archive -Path $pkg -DestinationPath "$pkg.zip" -Force
Write-Host "Built $pkg.zip"

if ($env:GITHUB_ENV) { "PKG_NAME=$pkg" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8 }
