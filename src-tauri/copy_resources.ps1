$ErrorActionPreference = "Stop"

$ScriptDir = $PSScriptRoot
$AssetsDir = Join-Path $ScriptDir "..\assets"
$ReleaseDir = Join-Path $ScriptDir "target\release"

Write-Host "Assets Dir: $AssetsDir"
Write-Host "Release Dir: $ReleaseDir"

if (-not (Test-Path $ReleaseDir)) {
    Write-Error "Release directory does not exist. Please build first."
}

# 1. Copy Root Files from assets to release root
$RootFiles = @("index.html", "transporter.html", "manifest.json")
foreach ($File in $RootFiles) {
    $Src = Join-Path $AssetsDir $File
    if (Test-Path $Src) {
        Copy-Item -Path $Src -Destination $ReleaseDir -Force
        Write-Host "Copied $File"
    } else {
        Write-Warning "File not found: $Src"
    }
}

# 2. Copy Root Folders from assets to release root
$RootFolders = @("dexopt", "static", "tiles")
foreach ($Folder in $RootFolders) {
    $Src = Join-Path $AssetsDir $Folder
    $Dest = Join-Path $ReleaseDir $Folder
    if (Test-Path $Src) {
        if (Test-Path $Dest) { Remove-Item $Dest -Recurse -Force }
        Copy-Item -Path $Src -Destination $ReleaseDir -Recurse -Force
        Write-Host "Copied folder $Folder"
    } else {
        Write-Warning "Folder not found: $Src"
    }
}

# 3. Handle mod folder
$ModSrc = Join-Path $AssetsDir "mod"
$ModDest = Join-Path $ReleaseDir "mod"

# Create mod dir
if (-not (Test-Path $ModDest)) { New-Item -ItemType Directory -Path $ModDest | Out-Null }

# 3a. Copy mod/data
$DataSrc = Join-Path $ModSrc "data"
$DataDest = Join-Path $ModDest "data"
if (Test-Path $DataSrc) {
    if (Test-Path $DataDest) { Remove-Item $DataDest -Recurse -Force }
    Copy-Item -Path $DataSrc -Destination $ModDest -Recurse -Force
    
    # Remove .pfx if any exist in data (unlikely but safe)
    Get-ChildItem -Path $DataDest -Include *.pfx -Recurse | Remove-Item -Force
    Write-Host "Copied mod/data (cleaned)"
}

# 3b. Copy mod/js (Bundles only)
$JsSrc = Join-Path $ModSrc "js"
$JsDest = Join-Path $ModDest "js"
if (-not (Test-Path $JsDest)) { New-Item -ItemType Directory -Path $JsDest | Out-Null }

if (Test-Path $JsSrc) {
    # Copy only .bundle.js and .LICENSE.txt
    # Use Join-Path for cross-platform compatibility
    $SearchPath = Join-Path $JsSrc "*"
    $JsFiles = Get-ChildItem -Path $SearchPath -Include "*.bundle.js", "*.LICENSE.txt" -File
    if ($JsFiles) {
        foreach ($File in $JsFiles) {
            Copy-Item -Path $File.FullName -Destination $JsDest -Force
            Write-Host "Copied $($File.Name)"
        }
        Write-Host "Copied mod/js bundles"
    } else {
        Write-Warning "No bundle files found in $JsSrc"
    }
}

Write-Host "Resource copy complete."
