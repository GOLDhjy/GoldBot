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
    $headers = @{ "User-Agent" = "goldbot-installer" }
    # 匿名请求限流 60 次/小时，设置 GITHUB_TOKEN 可提升限额并绕过 403。
    if ($env:GITHUB_TOKEN) {
        $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN"
    }
    try {
        $resp = Invoke-RestMethod -Uri $apiUrl -Headers $headers
        if (-not $resp.tag_name) {
            throw "No tag_name in GitHub API response."
        }
        return [string]$resp.tag_name
    } catch {
        Write-Host "GitHub API failed ($($_.Exception.Message)), falling back to redirect resolution..." -ForegroundColor DarkYellow
    }

    # 降级方案：/releases/latest 会 302 到 /releases/tag/<tag>，不受 API 匿名限流约束。
    # 用 .NET HttpWebRequest 而不是 Invoke-WebRequest，因为 PS 5.1 的
    # -MaximumRedirection 0 抛 InvalidOperationException 拿不到 Location 头。
    $releaseUrl = "https://github.com/$Repo/releases/latest"
    $location = $null
    try {
        $req = [System.Net.HttpWebRequest]::Create($releaseUrl)
        $req.AllowAutoRedirect = $false
        $req.UserAgent = "goldbot-installer"
        $req.Timeout = 15000
        $resp = $req.GetResponse()
        try {
            $location = $resp.Headers["Location"]
        } finally {
            $resp.Close()
        }
    } catch [System.Net.WebException] {
        # 3xx 不会抛错，这里只处理网络层异常；若带响应则尽力取 Location。
        if ($_.Exception.Response -and $_.Exception.Response.Headers) {
            $location = $_.Exception.Response.Headers["Location"]
        }
    } catch {
        Write-Host "Redirect resolution failed: $($_.Exception.Message)" -ForegroundColor DarkYellow
    }
    if ($location) {
        $tag = ([string]$location).TrimEnd("/") -split "/" | Select-Object -Last 1
        if ($tag -match "^v\d+(\.\d+)*$") {
            return $tag
        }
    }
    throw "Failed to resolve latest release tag from $apiUrl or $releaseUrl."
}

function Expand-ZipArchiveCompat {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ZipPath,
        [Parameter(Mandatory = $true)]
        [string]$Destination
    )
    # 旧版 .NET（< 4.7.1）上 Expand-Archive 会因缺少 3 参 ExtractToDirectory 报错，
    # 这里手动遍历条目解压，仅依赖 .NET 4.5 就有的 ExtractToFile。
    Add-Type -AssemblyName System.IO.Compression.FileSystem | Out-Null
    $zip = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        $root = [System.IO.Path]::GetFullPath($Destination)
        foreach ($entry in $zip.Entries) {
            # 防御 zip-slip：跳过包含路径穿越或绝对路径的条目。
            if ($entry.FullName -match "(^|/)\.\.(/|$)" -or $entry.FullName -match "^[a-zA-Z]:/" -or $entry.FullName.StartsWith("/")) {
                continue
            }
            $destPath = Join-Path $Destination $entry.FullName
            if (-not ([System.IO.Path]::GetFullPath($destPath)).StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
                continue
            }
            $destDir = Split-Path $destPath -Parent
            if (-not (Test-Path $destDir)) {
                New-Item -ItemType Directory -Force -Path $destDir | Out-Null
            }
            if ($entry.FullName.EndsWith("/")) {
                continue
            }
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destPath, $true)
        }
    } finally {
        $zip.Dispose()
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
        try {
            Expand-Archive -Path $zipPath -DestinationPath $tempRoot -Force
        } catch {
            Expand-ZipArchiveCompat -ZipPath $zipPath -Destination $tempRoot
        }

        $binary = Get-ChildItem -Path $tempRoot -Recurse -File -Filter "goldbot.exe" |
            Select-Object -First 1
        if (-not $binary) {
            throw "goldbot.exe not found in archive."
        }

        $dest = Join-Path $InstallDir "goldbot.exe"
        # 直接覆盖正在运行的 exe 会失败（文件被占用），而重命名运行中的 exe 是允许的，
        # 因此先把旧文件改名为 .old 备份再复制新文件。
        if (Test-Path $dest) {
            Remove-Item -Path "$dest.old" -Force -ErrorAction SilentlyContinue
            Move-Item -Path $dest -Destination "$dest.old" -Force
        }
        Copy-Item -Path $binary.FullName -Destination $dest -Force
        # 若旧 goldbot 进程仍在运行，删除备份会失败，留待下次 goldbot 启动时清理。
        Remove-Item -Path "$dest.old" -Force -ErrorAction SilentlyContinue
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
