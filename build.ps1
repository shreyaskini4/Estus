<#
.SYNOPSIS
    Builds the Estus release binary.

.DESCRIPTION
    Validates prerequisites (Cargo, smartctl, process locks), configures
    target directories, and compiles Estus with 'cargo build --release'.

.OUTPUTS
    Exit Codes:
      0 = Success
      1 = Build failed (cargo build returned non-zero)
      2 = Missing dependency (cargo not found)
      4 = Estus is currently running (causes file lock / OS error 5)
#>

param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$origLocation = Get-Location
Set-Location $PSScriptRoot

try {
    # 1. Check that cargo is available
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        $cargoDefaultPath = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
        if (Test-Path $cargoDefaultPath) {
            $cargoBinDir = Join-Path $env:USERPROFILE ".cargo\bin"
            $env:PATH = "$cargoBinDir;$env:PATH"
        }
    }

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host "[ERROR] Cargo was not found. Please run setup.bat first to install Rust and build tools." -ForegroundColor Red
        exit 2
    }

    # 2. Check if smartctl is available (do not stop or exit if missing)
    $smartctlFound = $false
    if (Get-Command smartctl -ErrorAction SilentlyContinue) {
        $smartctlFound = $true
    } elseif ($env:ProgramFiles -and (Test-Path (Join-Path $env:ProgramFiles "smartmontools\bin\smartctl.exe"))) {
        $smartctlFound = $true
    } elseif (${env:ProgramFiles(x86)} -and (Test-Path (Join-Path ${env:ProgramFiles(x86)} "smartmontools\bin\smartctl.exe"))) {
        $smartctlFound = $true
    }

    if (-not $smartctlFound) {
        Write-Host "[WARN] smartctl was not found. Estus will build, but drive SMART metrics require smartmontools." -ForegroundColor Yellow
    }

    # 3. Check if estus.exe is currently running
    if (Get-Process estus -ErrorAction SilentlyContinue) {
        Write-Host "[ERROR] estus.exe is currently running. Please right-click the Estus icon in the system tray and select 'Quit' before rebuilding, or the build will fail with 'Access is denied (os error 5)'." -ForegroundColor Red
        exit 4
    }

    # 4. Respect optional ESTUS_TARGET_DIR environment variable
    if (-not [string]::IsNullOrWhiteSpace($env:ESTUS_TARGET_DIR)) {
        $targetCandidate = $env:ESTUS_TARGET_DIR.Trim()
        $isValidTarget = $false
        try {
            if (Test-Path $targetCandidate) {
                $isValidTarget = $true
            } else {
                $qualifier = Split-Path $targetCandidate -Qualifier -ErrorAction SilentlyContinue
                if ($qualifier) {
                    if (Test-Path $qualifier) {
                        $isValidTarget = $true
                    }
                } else {
                    # Relative path or rootless path - current location/drive exists
                    $isValidTarget = $true
                }
            }
        } catch {
            $isValidTarget = $false
        }

        if ($isValidTarget) {
            $env:CARGO_TARGET_DIR = $targetCandidate
            Write-Host "[INFO] Using custom target directory from ESTUS_TARGET_DIR: $env:CARGO_TARGET_DIR" -ForegroundColor Cyan
        } else {
            Write-Host "[WARN] Custom ESTUS_TARGET_DIR '$env:ESTUS_TARGET_DIR' points to a non-existent drive or location. Falling back to default './target'." -ForegroundColor Yellow
            Remove-Item env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        }
    }

    # 5. Run cargo build --release
    if ($CargoArgs -and $CargoArgs.Count -gt 0) {
        cargo build --release @CargoArgs
    } else {
        cargo build --release
    }

    if ($LASTEXITCODE -ne 0) {
        Write-Host "[ERROR] Build failed." -ForegroundColor Red
        exit 1
    }

    # 6. Locate and display the built binary
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        $targetDir = $env:CARGO_TARGET_DIR
    } else {
        $targetDir = Join-Path $PSScriptRoot "target"
    }

    $exePath = Join-Path $targetDir "release\estus.exe"
    if (Test-Path $exePath) {
        $exeItem = Get-Item $exePath
        $sizeMB = [math]::Round($exeItem.Length / 1MB, 2)
        Write-Host "[OK] Build succeeded: $($exeItem.FullName) ($sizeMB MB)" -ForegroundColor Green
    } else {
        Write-Host "[OK] Build succeeded: $exePath" -ForegroundColor Green
    }

    # 7. Exit with code 0 on success
    exit 0
}
finally {
    Set-Location $origLocation
}
