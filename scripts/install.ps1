# GoldBot Windows Installer
# Usage:
#   irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
#   irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex; Install-GoldBot -Version v0.2.0

$ErrorActionPreference = "Stop"

function Get-LatestTag {
    param(
        [string]$Repo = "GOLDhjy/GoldBot"
    )
    $apiUrl = "https://api.github.com/repos/$Repo/releases/latest"
    try {
        $resp = Invoke-RestMethod -Uri $apiUrl -Headers @{ "User-Agent" = "goldbot-installer" }
        if (-not $resp.tag_name) {
            throw "No tag_name in GitHub API response."
        }
        return [string]$resp.tag_name
    } catch {
        throw "Failed to resolve latest release tag from $apiUrl. $($_.Exception.Message)"
    }
}

function Ensure-UserPathContains {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InstallDir
    )
    $registryPath = "HKCU:\Environment"
    $existingPath = (Get-ItemProperty -Path $registryPath -Name Path -ErrorAction SilentlyContinue).Path
    if ([string]::IsNullOrWhiteSpace($existingPath)) {
        $newPath = $InstallDir
    } elseif ($existingPath -notlike "*$InstallDir*") {
        $newPath = "$existingPath;$InstallDir"
    } else {
        return
    }
    Set-ItemProperty -Path $registryPath -Name Path -Value $newPath
    Write-Host "Added $InstallDir to user PATH. Restart terminal to take effect." -ForegroundColor Yellow
}

function Install-GoldBot {
    param(
        [string]$Version = "latest",
        [string]$Repo = "GOLDhjy/GoldBot",
        [string]$InstallDir = "$env:USERPROFILE\.goldbot\bin"
    )

    $tag = $Version
    if ($tag -eq "latest") {
        $tag = Get-LatestTag -Repo $Repo
    }
    if (-not $tag.StartsWith("v")) {
        throw "Version must be a release tag like v0.2.0. Got: $tag"
    }

    $asset = "goldbot-$tag-windows-x86_64.zip"
    $zipUrl = "https://github.com/$Repo/releases/download/$tag/$asset"

    $tempRoot = Join-Path $env:TEMP ("goldbot-install-" + [System.Guid]::NewGuid().ToString("N"))
    $zipPath = Join-Path $tempRoot $asset
    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    try {
        Write-Host "Downloading $asset ..." -ForegroundColor Cyan
        Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing

        Write-Host "Extracting archive..." -ForegroundColor Cyan
        Expand-Archive -Path $zipPath -DestinationPath $tempRoot -Force

        $binary = Get-ChildItem -Path $tempRoot -Recurse -File -Filter "goldbot.exe" |
            Select-Object -First 1
        if (-not $binary) {
            throw "goldbot.exe not found in archive."
        }

        Copy-Item -Path $binary.FullName -Destination (Join-Path $InstallDir "goldbot.exe") -Force
        $env:PATH = "$InstallDir;$env:PATH"
        Ensure-UserPathContains -InstallDir $InstallDir

        Write-Host "`nGoldBot installed: $InstallDir\goldbot.exe" -ForegroundColor Green
        Write-Host "Run 'goldbot' to start." -ForegroundColor Green
    } finally {
        Remove-Item -Path $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Execute with defaults when piped via iex.
Install-GoldBot
