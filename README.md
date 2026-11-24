# Estus

[![GitHub Release](https://img.shields.io/github/v/release/shreyaskini4/Estus)](https://github.com/shreyaskini4/Estus/releases)

A lightweight, native Windows desktop application written in Rust that monitors and analyzes what is wearing out your NVMe SSD.

Single small executable. Negligible CPU and RAM overhead. Zero web technology (no Electron, no WebView, no Chromium).

---

## Download and Quick Start

Pre-built releases are available on the [GitHub Releases](https://github.com/shreyaskini4/Estus/releases) page.

1. **Download** the latest `Estus-<version>-windows-x64.zip` and the `.sha256` checksum file from [Releases](https://github.com/shreyaskini4/Estus/releases).
2. **Verify integrity** (optional) in PowerShell:
   ```powershell
   Get-FileHash .\Estus-*-windows-x64.zip -Algorithm SHA256
   ```
   Compare the output against the checksum in the `.sha256` file.
3. **Extract** the zip to any folder.
4. **Install smartmontools** if not already installed:
   ```powershell
   winget install smartmontools.smartmontools
   ```
5. **Run** `estus.exe`. The embedded manifest automatically requests administrator elevation via UAC.

> [!NOTE]
> **Windows SmartScreen:** Because Estus is an open-source tool without a commercial code-signing certificate, Windows SmartScreen may show a warning ("Windows protected your PC"). Click "More info", then click "Run anyway".

---

## Features

### Drive Monitoring
- Automatically discovers all installed SSDs using `smartctl --scan -j` and identifies each drive by serial number and model.
- Supports both NVMe SMART health telemetry and SATA wear leveling / LBA written attributes.
- Switch between individual drives (with customizable nicknames) and an aggregated all-drives view.

### Health Metrics
- Percentage Used (wear %)
- Total Host Writes (TB)
- Writes Today (GB)
- Projected Lifespan (days until 100% wear)
- Available Spare capacity
- Power-On Hours and Power Cycles

### Write Analytics
- 30-day interactive bar chart of daily write volume per drive or aggregated across all drives.
- Top writing processes with toggles for Today, Last 7 Days, and Last 30 Days.
- Configurable daily alert threshold (default: 100 GB) with desktop toast notifications, rate-limited to once per day per drive.

### System Integration
- Runs as a native GUI application with no console window.
- Minimizes to the system tray with Show and Quit context menu options.
- Autostart with Windows via Task Scheduler, starting silently to the tray on logon.

### Diagnostics and Storage
- Multi-stage smartctl auto-detection: custom user path, system PATH, Program Files, and Windows Registry.
- Built-in Diagnostics panel for path browsing, re-detection, and inspecting raw output.
- Embedded SQLite database (WAL mode) with automatic schema migration and 90-day data pruning.
- Rotating file logger (1 MB limit) with panic hook for crash diagnostics.

---

## How the Numbers Are Measured

### NVMe SMART Metrics

Estus queries `smartctl -j -a` every 10 minutes in the background.

- **Data Units Written and Read** are read from the NVMe `nvme_smart_health_information_log`. Per the NVM Express specification, 1 Data Unit = 1,000 x 512 bytes = 512,000 bytes. Total host bytes:

$$\text{Host Bytes Written} = \text{data\_units\_written} \times 512{,}000$$

- **Percentage Used** is the vendor-reported wear indicator. 0% is brand new; 100% means the rated endurance (TBW) has been reached (not necessarily immediate failure).
- **Available Spare** is the remaining reserve flash capacity for replacing degraded blocks.

### Per-Process Write Tracking

Every 30 seconds, the background thread reads `total_written_bytes` for each process using `sysinfo` with minimal refresh flags.

- Deltas are computed as current minus previous reading.
- If a PID is reused (current < previous), the negative delta is discarded and the baseline resets.
- Write deltas are aggregated by process name so multi-instance applications are accurately represented.

### Lifespan Projection

- **Average Daily Writes**: 7-day rolling average in GB.

$$\text{Average GB/Day} = \frac{\sum_{i=1}^{N} \text{Daily Writes}_i}{N} \quad (N \le 7)$$

- **Projected Days to 100% Wear**:

$$\text{Bytes per 1\% Wear} = \frac{\text{Total Host Bytes Written}}{\text{Percentage Used}}$$

$$\text{Remaining Bytes} = (100 - \text{Percentage Used}) \times \text{Bytes per 1\% Wear}$$

$$\text{Projected Days} = \frac{\text{Remaining Bytes}}{\text{Average Daily Write Rate}}$$

If the drive is at 0% wear, the projection displays "Indefinite" until a measurable baseline accumulates.

- **Writes per Power-On Hour**:

$$\text{GB/Hour} = \frac{\text{Total Host Bytes Written}}{10^9 \times \text{Power-On Hours}}$$

---

## Architecture

```
src/
  main.rs          Entry point, eframe init, tray orchestration, panic hook
  smart.rs         smartctl resolution, multi-drive discovery, SMART parsing
  procs.rs         sysinfo process disk polling, PID reuse delta calculation
  db.rs            SQLite storage (WAL mode), per-drive snapshots and deltas
  analytics.rs     Pure functions for write rates, lifespan, and thresholds
  app_core.rs      Background threads, tray icon, notifications, settings
  ui.rs            egui dashboard, drive selector, 30-day plot, diagnostics
  logger.rs        Rotating file logger and crash formatting
  process_util.rs  Silent subprocess creation with CREATE_NO_WINDOW
```

### Dependencies

| Category | Crates |
| :--- | :--- |
| UI and Graphics | `eframe`, `egui`, `egui_plot` |
| System Monitoring | `sysinfo` |
| Storage | `rusqlite` (bundled) |
| Desktop Integration | `tray-icon`, `notify-rust` |
| Serialization | `serde`, `serde_json`, `directories` |
| Time and Errors | `chrono`, `anyhow` |
| Build Manifest | `embed-manifest` |

---

## Building from Source

Estus builds on any 64-bit Windows machine. Use the automated scripts or configure manually.

### Quick Path (Automated)

1. Double-click `setup.bat` (or run `.\setup.ps1` as administrator). This installs Git, Rust (MSVC toolchain), Visual Studio C++ Build Tools, and smartmontools via `winget`.
2. Run `.\build.ps1` to compile the release binary.

### Manual Path

1. Install Rust via [rustup.rs](https://rustup.rs/) with the MSVC toolchain:
   ```powershell
   winget install --id Rustlang.Rustup
   rustup default stable-x86_64-pc-windows-msvc
   ```
2. Install Visual Studio Build Tools with the "Desktop development with C++" workload:
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools
   ```
3. Install smartmontools:
   ```powershell
   winget install smartmontools.smartmontools
   ```
4. Build:
   ```powershell
   cargo build --release
   ```

The output binary is at `target\release\estus.exe`.

### Custom Target Directory

To store build artifacts on a separate drive:
```powershell
$env:ESTUS_TARGET_DIR = "D:\cargo-target"
.\build.ps1
```
Or with Cargo directly:
```powershell
$env:CARGO_TARGET_DIR = "D:\cargo-target"
cargo build --release
```

### Script Exit Codes

| Code | Meaning |
| :--- | :--- |
| 0 | Success |
| 1 | Build or general failure |
| 2 | Missing dependency |
| 3 | Elevation required |
| 4 | Estus is running (file locked) |

> [!IMPORTANT]
> Always use `--release` for all cargo commands (`build`, `check`, `clippy`, `test`). Debug builds are intentionally disabled in `Cargo.toml` to avoid bloated artifacts.

---

## Notes

- Estus embeds a Windows manifest that requests administrator privileges. This is required to query NVMe controllers and configure Task Scheduler autostart.
- If Estus is running in the tray, quit it before rebuilding. The locked binary causes `Access is denied (os error 5)`.
- All dependencies (including SQLite) are statically compiled into a single `estus.exe`. No external DLLs or runtimes are needed.
- The application runs under the Windows GUI subsystem with no console window.
- You can place `estus.exe` anywhere on your machine. Logs are stored at `%LOCALAPPDATA%\estus\Estus\data\estus.log`.

---

## Development

Always include `--release` for all cargo operations:

```powershell
cargo check --release
cargo test --release
cargo clippy --release --all-targets -- -D warnings
```

---

## Troubleshooting

| Problem | Solution |
| :--- | :--- |
| `cargo` not recognized | Run `setup.bat`, or add `%USERPROFILE%\.cargo\bin` to your PATH and restart your terminal. |
| `link.exe` not found | Install Visual Studio C++ Build Tools with the "Desktop development with C++" workload. |
| Access is denied (os error 5) | Quit Estus from the system tray before rebuilding. |
| smartctl not detected | Run `winget install smartmontools.smartmontools`, or use the Browse button in the Estus Diagnostics panel. |
| `winget` not recognized | Install "App Installer" from the Microsoft Store. |
| Running scripts is disabled | Use `setup.bat` (bypasses execution policy), or run `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass`. |

---

## Releasing

For maintainer release instructions, see [RELEASING.md](RELEASING.md).

---

## License

MIT OR Apache-2.0
