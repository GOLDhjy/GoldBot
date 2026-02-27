# GoldBot Windows Installer
# Usage:
#   irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
#   # For Windows PowerShell 5.1 TLS issues:
#   [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; iwr "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" -UseBasicParsing | iex
#   # To also persist machine PATH, run installer in an Administrator PowerShell.
#   irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex; Install-GoldBot -Version v0.2.0

$ErrorActionPreference = "Stop"

function Enable-Tls12 {
    try {
        $current = [System.Net.ServicePointManager]::SecurityProtocol
        if (($current -band [System.Net.SecurityProtocolType]::Tls12) -eq 0) {
            [System.Net.ServicePointManager]::SecurityProtocol = $current -bor [System.Net.SecurityProtocolType]::Tls12
        }
    } catch {
        # Ignore on platforms where ServicePointManager is not applicable.
    }
}

function Invoke-WebRequestCompat {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Uri,
        [Parameter(Mandatory = $true)]
        [string]$OutFile,
        [int]$Retries = 3
    )

    $supportsUseBasicParsing = (Get-Command Invoke-WebRequest).Parameters.ContainsKey("UseBasicParsing")
    for ($attempt = 1; $attempt -le $Retries; $attempt++) {
        try {
            if ($supportsUseBasicParsing) {
                Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing
            } else {
                Invoke-WebRequest -Uri $Uri -OutFile $OutFile
            }
            return
        } catch {
            if ($attempt -ge $Retries) {
                throw
            }
            Start-Sleep -Seconds $attempt
        }
    }
}

Enable-Tls12

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

function Test-IsAdministrator {
    try {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

function Path-ContainsDir {
    param(
        [string]$PathValue,
        [string]$Dir
    )
    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $false
    }
    $needle = $Dir.Trim().TrimEnd('\')
    return ($PathValue -split ';' |
        ForEach-Object { $_.Trim().TrimEnd('\') } |
        Where-Object { $_ -ieq $needle } |
        Select-Object -First 1) -ne $null
}

function Ensure-UserPathContains {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InstallDir
    )
    $existingPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
    if ([string]::IsNullOrWhiteSpace($existingPath)) {
        $newPath = $InstallDir
    } elseif (-not (Path-ContainsDir -PathValue $existingPath -Dir $InstallDir)) {
        $newPath = "$existingPath;$InstallDir"
    } else {
        return
    }
    [Environment]::SetEnvironmentVariable("Path", $newPath, [EnvironmentVariableTarget]::User)
    Write-Host "Added $InstallDir to user PATH. Restart terminal to take effect." -ForegroundColor Yellow
}

function Ensure-SystemPathContains {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InstallDir
    )

    if (-not (Test-IsAdministrator)) {
        Write-Host "Skipped system PATH update (run PowerShell as Administrator to enable)." -ForegroundColor DarkYellow
        return
    }

    try {
        $existingPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::Machine)
        if ([string]::IsNullOrWhiteSpace($existingPath)) {
            $newPath = $InstallDir
        } elseif (-not (Path-ContainsDir -PathValue $existingPath -Dir $InstallDir)) {
            $newPath = "$existingPath;$InstallDir"
        } else {
            return
        }
        [Environment]::SetEnvironmentVariable("Path", $newPath, [EnvironmentVariableTarget]::Machine)
        Write-Host "Added $InstallDir to system PATH (Machine)." -ForegroundColor Yellow
    } catch {
        Write-Host "Failed to update system PATH: $($_.Exception.Message)" -ForegroundColor DarkYellow
    }
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
        Invoke-WebRequestCompat -Uri $zipUrl -OutFile $zipPath

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
        Ensure-SystemPathContains -InstallDir $InstallDir

        Write-Host "`nGoldBot installed: $InstallDir\goldbot.exe" -ForegroundColor Green
        Write-Host "Run 'goldbot' to start." -ForegroundColor Green
    } finally {
        Remove-Item -Path $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Execute with defaults when piped via iex.
Install-GoldBot
