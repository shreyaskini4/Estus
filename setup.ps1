<#
.SYNOPSIS
    One-click dependency installer and verification script for Estus.

.DESCRIPTION
    Installs missing build and runtime dependencies for Estus (Git, Rust, Cargo,
    Visual Studio 2022 C++ Build Tools, and smartmontools) using winget.
    When run with -Check, inspects existing dependencies and outputs a verification report.

.PARAMETER Check
    When specified, inspects and reports dependency status without installing anything
    or prompting for elevation. Returns 0 if all dependencies are present, or 2 if any are missing.

.EXITCODES
    0 = Success (all tools installed / all verified present in -Check / build succeeded)
    1 = Installation error / general script failure
    2 = Missing required dependency (winget missing, or tools missing when run with -Check)
    3 = Elevation required / canceled by user
#>

[CmdletBinding()]
param(
    [switch]$Check
)

# ---------------------------------------------------------------------------
# Path & Environment Refresh Helper
# ---------------------------------------------------------------------------
function Update-SessionPath {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath    = [Environment]::GetEnvironmentVariable("Path", "User")
    $combined    = "$machinePath;$userPath"

    $pathList = ($combined -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    $cargoBin       = "$env:USERPROFILE\.cargo\bin"
    $smartctlBin    = "${env:ProgramFiles}\smartmontools\bin"
    $smartctlBinX86 = "${env:ProgramFiles(x86)}\smartmontools\bin"

    if ($pathList -notcontains $cargoBin) {
        $pathList += $cargoBin
    }
    if ($pathList -notcontains $smartctlBin) {
        $pathList += $smartctlBin
    }
    if ($pathList -notcontains $smartctlBinX86 -and (Test-Path $smartctlBinX86)) {
        $pathList += $smartctlBinX86
    }

    $env:Path = ($pathList -join ';')
}

# ---------------------------------------------------------------------------
# Tool Detection Functions
# ---------------------------------------------------------------------------
function Get-GitInfo {
    $cmd = Get-Command git.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        try {
            $ver = & git.exe --version 2>$null
            if ($ver) {
                return @{
                    Installed = $true
                    Version   = $ver.Trim()
                    Path      = $cmd.Source
                }
            }
        } catch {}
    }
    return @{ Installed = $false; Version = "Not found"; Path = $null }
}

function Get-RustcInfo {
    $exePath = $null
    $cmd = Get-Command rustc.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        $exePath = $cmd.Source
    } elseif (Test-Path "$env:USERPROFILE\.cargo\bin\rustc.exe") {
        $exePath = "$env:USERPROFILE\.cargo\bin\rustc.exe"
    }

    if ($exePath) {
        try {
            $ver = & $exePath --version 2>$null
            if ($ver) {
                return @{
                    Installed = $true
                    Version   = $ver.Trim()
                    Path      = $exePath
                }
            }
        } catch {}
    }
    return @{ Installed = $false; Version = "Not found"; Path = $null }
}

function Get-CargoInfo {
    $exePath = $null
    $cmd = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        $exePath = $cmd.Source
    } elseif (Test-Path "$env:USERPROFILE\.cargo\bin\cargo.exe") {
        $exePath = "$env:USERPROFILE\.cargo\bin\cargo.exe"
    }

    if ($exePath) {
        try {
            $ver = & $exePath --version 2>$null
            if ($ver) {
                return @{
                    Installed = $true
                    Version   = $ver.Trim()
                    Path      = $exePath
                }
            }
        } catch {}
    }
    return @{ Installed = $false; Version = "Not found"; Path = $null }
}

function Get-MSVCInfo {
    $vswherePath = $null
    $cmd = Get-Command vswhere.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        $vswherePath = $cmd.Source
    } else {
        $defaultVsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
        if (Test-Path $defaultVsWhere) {
            $vswherePath = $defaultVsWhere
        }
    }

    $installed   = $false
    $vsVersion   = $null
    $linkPath    = $null
    $linkVersion = $null

    if ($vswherePath) {
        try {
            $vsCheck = & $vswherePath -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -latest -property installationVersion 2>$null
            if ($vsCheck) {
                $installed = $true
                $vsVersion = $vsCheck.Trim()
                $links = & $vswherePath -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -latest -find "VC\Tools\MSVC\**\bin\Hostx64\x64\link.exe" 2>$null
                if (-not $links) {
                    $links = & $vswherePath -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -latest -find "VC\Tools\MSVC\**\bin\Hostx86\x64\link.exe" 2>$null
                }
                if ($links) {
                    $linkPath = ($links | Select-Object -Last 1).Trim()
                }
            }
        } catch {}
    }

    if (-not $linkPath) {
        $linkCmd = Get-Command link.exe -ErrorAction SilentlyContinue
        if ($linkCmd) {
            $linkPath = $linkCmd.Source
            $installed = $true
        }
    }

    if ($linkPath -and (Test-Path $linkPath)) {
        try {
            $fileVer = (Get-Item $linkPath).VersionInfo.FileVersion
            if ($fileVer) {
                $linkVersion = $fileVer.Trim()
            }
        } catch {}
    }

    return @{
        Installed   = $installed
        VSVersion   = $vsVersion
        LinkPath    = $linkPath
        LinkVersion = if ($linkVersion) { $linkVersion } elseif ($installed) { "Installed (MSVC Tools)" } else { "Not found" }
    }
}

function Get-SmartctlInfo {
    $smartctlPath = $null
    $cmd = Get-Command smartctl.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        $smartctlPath = $cmd.Source
    } elseif (Test-Path "${env:ProgramFiles}\smartmontools\bin\smartctl.exe") {
        $smartctlPath = "${env:ProgramFiles}\smartmontools\bin\smartctl.exe"
    } elseif (Test-Path "${env:ProgramFiles(x86)}\smartmontools\bin\smartctl.exe") {
        $smartctlPath = "${env:ProgramFiles(x86)}\smartmontools\bin\smartctl.exe"
    }

    if ($smartctlPath) {
        try {
            $verOut = & $smartctlPath --version 2>$null | Select-Object -First 1
            if ($verOut) {
                return @{
                    Installed = $true
                    Version   = $verOut.Trim()
                    Path      = $smartctlPath
                }
            }
        } catch {}
    }
    return @{ Installed = $false; Version = "Not found"; Path = $null }
}

# ---------------------------------------------------------------------------
# Report Output Function
# ---------------------------------------------------------------------------
function Show-VerificationReport {
    param(
        [hashtable]$Git,
        [hashtable]$Rustc,
        [hashtable]$Cargo,
        [hashtable]$MSVC,
        [hashtable]$Smartctl,
        [System.Collections.Generic.List[string]]$FailedList = $null
    )

    Write-Host ""
    Write-Host "=================================================================" -ForegroundColor DarkGray
    Write-Host "                  DEPENDENCY VERIFICATION REPORT                 " -ForegroundColor White
    Write-Host "=================================================================" -ForegroundColor DarkGray
    Write-Host ("{0,-12} {1,-22} {2}" -f " STATUS", "COMPONENT", "VERSION / DETAILS")
    Write-Host "-----------------------------------------------------------------" -ForegroundColor DarkGray

    # Git
    if ($Git.Installed) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    } else {
        Write-Host " [MISSING]  " -ForegroundColor Red -NoNewline
    }
    Write-Host ("{0,-22} {1}" -f "Git", $Git.Version)

    # rustc
    if ($Rustc.Installed) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    } else {
        Write-Host " [MISSING]  " -ForegroundColor Red -NoNewline
    }
    Write-Host ("{0,-22} {1}" -f "rustc", $Rustc.Version)

    # cargo
    if ($Cargo.Installed) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    } else {
        Write-Host " [MISSING]  " -ForegroundColor Red -NoNewline
    }
    Write-Host ("{0,-22} {1}" -f "cargo", $Cargo.Version)

    # link.exe (MSVC C++ Build Tools)
    if ($MSVC.Installed -and $MSVC.LinkPath) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-22} {1}" -f "link.exe (MSVC)", $MSVC.LinkVersion)
    } elseif ($MSVC.Installed) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host ("{0,-22} {1}" -f "link.exe (MSVC)", "Installed (MSVC Tools)")
    } else {
        Write-Host " [MISSING]  " -ForegroundColor Red -NoNewline
        Write-Host ("{0,-22} {1}" -f "link.exe (MSVC)", "MSVC C++ Build Tools not found")
    }

    # smartctl
    if ($Smartctl.Installed) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    } else {
        Write-Host " [MISSING]  " -ForegroundColor Red -NoNewline
    }
    Write-Host ("{0,-22} {1}" -f "smartctl", $Smartctl.Version)

    Write-Host "=================================================================" -ForegroundColor DarkGray
    Write-Host ""

    if ($FailedList -and $FailedList.Count -gt 0) {
        Write-Host "The following installations encountered errors:" -ForegroundColor Red
        foreach ($f in $FailedList) {
            Write-Host " [FAILED]   " -ForegroundColor Red -NoNewline
            Write-Host $f
        }
        Write-Host ""
    }
}

# ---------------------------------------------------------------------------
# MAIN SCRIPT EXECUTION
# ---------------------------------------------------------------------------

# Update PATH from registry and ensure standard tool paths are accessible
Update-SessionPath

if ($Check) {
    # -----------------------------------------------------------------------
    # -Check Mode: Inspection Only
    # -----------------------------------------------------------------------
    $gitInfo      = Get-GitInfo
    $rustcInfo    = Get-RustcInfo
    $cargoInfo    = Get-CargoInfo
    $msvcInfo     = Get-MSVCInfo
    $smartctlInfo = Get-SmartctlInfo

    Show-VerificationReport -Git $gitInfo -Rustc $rustcInfo -Cargo $cargoInfo -MSVC $msvcInfo -Smartctl $smartctlInfo

    $allPresent = $gitInfo.Installed -and $rustcInfo.Installed -and $cargoInfo.Installed -and $msvcInfo.Installed -and $smartctlInfo.Installed

    if ($allPresent) {
        Write-Host "[OK] All required dependencies are present." -ForegroundColor Green
        exit 0
    } else {
        Write-Host "[MISSING] One or more required dependencies are missing. Run setup.bat or setup.ps1 without -Check to install." -ForegroundColor Red
        exit 2
    }
}

# ---------------------------------------------------------------------------
# Installation / Setup Mode
# ---------------------------------------------------------------------------

# Step 1: Self-elevation check
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Administrative permissions are required to install dependencies." -ForegroundColor Cyan
    Write-Host "Relaunching elevated PowerShell prompt..." -ForegroundColor Cyan

    $scriptPath = if ($PSCommandPath) { $PSCommandPath } else { Join-Path $PSScriptRoot "setup.ps1" }
    try {
        $proc = Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$scriptPath`"" -PassThru -ErrorAction Stop
        exit 0
    } catch {
        Write-Host "[FAILED] Administrator elevation was canceled or failed: $_" -ForegroundColor Red
        exit 3
    }
}

# Step 2: Verify winget is available
$wingetCmd = Get-Command winget.exe -ErrorAction SilentlyContinue
if (-not $wingetCmd) {
    $defaultWinget = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\winget.exe"
    if (Test-Path $defaultWinget) {
        $wingetCmd = $defaultWinget
    }
}

if (-not $wingetCmd) {
    Write-Host "[MISSING] Windows Package Manager (winget) is not installed." -ForegroundColor Red
    Write-Host ""
    Write-Host "To install winget:" -ForegroundColor Yellow
    Write-Host "  1. Install or update 'App Installer' from the Microsoft Store, or"
    Write-Host "  2. Download directly from: https://aka.ms/getwinget"
    Write-Host ""
    exit 2
}

# Step 3: Inspect current environment and install ONLY what is missing
Write-Host "Checking installed tools..." -ForegroundColor Cyan
Write-Host ""

$failedInstalls  = [System.Collections.Generic.List[string]]::new()
$newlyInstalled  = $false

# 3a. Git
$gitInfo = Get-GitInfo
if ($gitInfo.Installed) {
    Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    Write-Host "Git is already installed ($($gitInfo.Version))"
} else {
    Write-Host " [INSTALLING] " -ForegroundColor Cyan -NoNewline
    Write-Host "Installing Git via winget..."
    & winget install --id Git.Git -e --silent --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -eq 0) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host "Successfully installed Git."
        $newlyInstalled = $true
    } else {
        Write-Host " [FAILED]   " -ForegroundColor Red -NoNewline
        Write-Host "Failed to install Git (winget exit code: $LASTEXITCODE)."
        $failedInstalls.Add("Git (winget exit code: $LASTEXITCODE)")
    }
}

# 3b. Rust & Cargo
$rustcInfo = Get-RustcInfo
$cargoInfo = Get-CargoInfo
if ($rustcInfo.Installed -and $cargoInfo.Installed) {
    Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    Write-Host "Rust & Cargo are already installed ($($rustcInfo.Version))"
} else {
    Write-Host " [INSTALLING] " -ForegroundColor Cyan -NoNewline
    Write-Host "Installing Rustup via winget..."
    & winget install --id Rustlang.Rustup -e --silent --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -eq 0) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host "Successfully installed Rustup."
        $newlyInstalled = $true

        Update-SessionPath
        Write-Host "Configuring Rust default toolchain (stable-x86_64-pc-windows-msvc)..." -ForegroundColor Cyan
        $rustupCmd = Get-Command rustup.exe -ErrorAction SilentlyContinue
        if ($rustupCmd) {
            & rustup.exe default stable-x86_64-pc-windows-msvc
        } elseif (Test-Path "$env:USERPROFILE\.cargo\bin\rustup.exe") {
            & "$env:USERPROFILE\.cargo\bin\rustup.exe" default stable-x86_64-pc-windows-msvc
        }
    } else {
        Write-Host " [FAILED]   " -ForegroundColor Red -NoNewline
        Write-Host "Failed to install Rustup (winget exit code: $LASTEXITCODE)."
        $failedInstalls.Add("Rustup (winget exit code: $LASTEXITCODE)")
    }
}

# 3c. Visual Studio 2022 C++ Build Tools
$msvcInfo = Get-MSVCInfo
if ($msvcInfo.Installed) {
    Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    Write-Host "Visual Studio C++ Build Tools are already installed"
} else {
    Write-Host ""
    Write-Host "=================================================================" -ForegroundColor Yellow
    Write-Host " [WARN] Visual Studio 2022 C++ Build Tools is missing!" -ForegroundColor Yellow
    Write-Host " WARNING: This is a large download (several GB) and will take time." -ForegroundColor Yellow
    Write-Host " Please keep this window open while the installation proceeds." -ForegroundColor Yellow
    Write-Host "=================================================================" -ForegroundColor Yellow
    Write-Host ""
    Write-Host " [INSTALLING] " -ForegroundColor Cyan -NoNewline
    Write-Host "Installing Visual Studio 2022 C++ Build Tools via winget..."
    & winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    if ($LASTEXITCODE -eq 0) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host "Successfully installed Visual Studio 2022 C++ Build Tools."
        $newlyInstalled = $true
    } else {
        Write-Host " [FAILED]   " -ForegroundColor Red -NoNewline
        Write-Host "Failed to install Visual Studio 2022 C++ Build Tools (winget exit code: $LASTEXITCODE)."
        $failedInstalls.Add("Visual Studio 2022 C++ Build Tools (winget exit code: $LASTEXITCODE)")
    }
}

# 3d. smartmontools
$smartctlInfo = Get-SmartctlInfo
if ($smartctlInfo.Installed) {
    Write-Host " [OK]       " -ForegroundColor Green -NoNewline
    Write-Host "smartmontools is already installed ($($smartctlInfo.Version))"
} else {
    Write-Host " [INSTALLING] " -ForegroundColor Cyan -NoNewline
    Write-Host "Installing smartmontools via winget..."
    & winget install --id smartmontools.smartmontools -e --silent --accept-package-agreements --accept-source-agreements
    $smartctlExit = $LASTEXITCODE
    if ($smartctlExit -ne 0) {
        Write-Host " [WARN] Package ID 'smartmontools.smartmontools' exited with code $smartctlExit. Retrying with fallback name..." -ForegroundColor Yellow
        & winget install smartmontools --silent --accept-package-agreements --accept-source-agreements
        $smartctlExit = $LASTEXITCODE
    }
    if ($smartctlExit -eq 0) {
        Write-Host " [OK]       " -ForegroundColor Green -NoNewline
        Write-Host "Successfully installed smartmontools."
        $newlyInstalled = $true
    } else {
        Write-Host " [FAILED]   " -ForegroundColor Red -NoNewline
        Write-Host "Failed to install smartmontools (winget exit code: $smartctlExit)."
        $failedInstalls.Add("smartmontools (winget exit code: $smartctlExit)")
    }
}

# Step 4: Refresh PATH after installations
Update-SessionPath

# Step 5 & 6: Final Verification Report
$gitFinal      = Get-GitInfo
$rustcFinal    = Get-RustcInfo
$cargoFinal    = Get-CargoInfo
$msvcFinal     = Get-MSVCInfo
$smartctlFinal = Get-SmartctlInfo

Show-VerificationReport -Git $gitFinal -Rustc $rustcFinal -Cargo $cargoFinal -MSVC $msvcFinal -Smartctl $smartctlFinal -FailedList $failedInstalls

# Check if any installs failed or tools remain missing
$allPresentFinal = $gitFinal.Installed -and $rustcFinal.Installed -and $cargoFinal.Installed -and $msvcFinal.Installed -and $smartctlFinal.Installed
if ($failedInstalls.Count -gt 0 -or -not $allPresentFinal) {
    Write-Host "[FAILED] Dependency installation failed or some required tools are still missing." -ForegroundColor Red
    exit 1
}

# Step 8: Notify user if environment variables need propagation
if ($newlyInstalled) {
    Write-Host "Notice: One or more dependencies were newly installed." -ForegroundColor Cyan
    Write-Host "Please close and reopen your terminal after this script finishes so new environment variables propagate globally." -ForegroundColor Cyan
    Write-Host ""
}

# Step 7: Prompt user to build Estus
$buildChoice = Read-Host "Build Estus now? (Y/N)"
if ($buildChoice -match '^[Yy]') {
    $buildScript = Join-Path $PSScriptRoot "build.ps1"
    if (Test-Path $buildScript) {
        Write-Host ""
        Write-Host "Launching build.ps1..." -ForegroundColor Cyan
        & $buildScript
        if ($LASTEXITCODE -ne $null) { exit $LASTEXITCODE } else { exit 0 }
    } else {
        Write-Host "[WARN] build.ps1 was not found in $PSScriptRoot." -ForegroundColor Yellow
        exit 0
    }
} else {
    Write-Host "Build skipped. Setup completed successfully." -ForegroundColor Green
    exit 0
}
