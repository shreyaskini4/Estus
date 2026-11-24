# Estus

[![GitHub Release](https://img.shields.io/github/v/release/shreyaskini4/Estus)](https://github.com/shreyaskini4/Estus/releases)

A lightweight, native Windows desktop application written in Rust that monitors and analyzes what is wearing out your NVMe SSD.

Designed with a focus on efficiency: **single small executable**, **negligible CPU and RAM overhead**, **zero web technology** (no Electron, no WebView, no Chromium).

---

## Download & Quick Start

Pre-built releases are available on the [GitHub Releases](https://github.com/shreyaskini4/Estus/releases) page:

1. **Download**: Go to [GitHub Releases](https://github.com/shreyaskini4/Estus/releases) and download the latest `Estus-<version>-windows-x64.zip` and the `.sha256` checksum file.
2. **Verify (Optional)**: Verify SHA256 integrity in PowerShell:
   ```powershell
   Get-FileHash .\Estus-*-windows-x64.zip -Algorithm SHA256
   ```
   Compare the output against the checksum in `.sha256`.
3. **Extract**: Extract the zip file to any folder on your machine.
4. **Install `smartmontools`** (if not already installed):
   ```powershell
   winget install smartmontools.smartmontools
   ```
   *(or `winget install smartmontools`)*
5. **Run**: Right-click `estus.exe` and select **"Run as administrator"** (or double-click; Estus embeds a manifest that automatically requests administrator elevation via UAC).

> [!NOTE]
> **Windows SmartScreen**: Because Estus is an open-source tool without a commercial code-signing certificate, Windows SmartScreen may show an initial warning banner ("Windows protected your PC"). Click **"More info"**, then click **"Run anyway"**.

---

## Features

- **Multi-SSD Monitoring (NVMe & SATA)**: Automatically discovers all installed solid-state drives using `smartctl --scan -j` and identifies them uniquely by `serial + model`. Supports both NVMe health telemetry and SATA SSD wear leveling/LBA written attributes.
- **Drive Selector & Global View**: Switch seamlessly between individual connected drives (with customizable nicknames) and a consolidated `🌐 [ All drives ]` view aggregating total host writes and daily write volume.
- **Top SSD Health Metrics**: Real-time view of **Percentage Used (Wear %)**, **Total Host Writes (TB)**, **Writes Today (GB)**, and **Projected Lifespan (Days until 100% wear)**.
- **30-Day Daily Write Trends**: Interactive bar chart powered by `egui_plot` displaying daily write volume in GB across the last 30 days per drive or aggregated across all drives.
- **Top Writing Processes**: Identifies which applications are writing the most data across the system, with quick toggles for **Today**, **Last 7 Days**, and **Last 30 Days**.
- **Configurable Daily Alerts**: Desktop toast notifications (via `notify-rust`) when daily write volume exceeds your configurable threshold (default: 100 GB). Alerts are evaluated per drive and rate-limited to at most once per day.
- **System Tray Integration & Silent Window**: Runs natively as a GUI application (`windows_subsystem = "windows"`) with zero command prompt flashing (`CREATE_NO_WINDOW`). Minimizes cleanly to the system tray (`tray-icon`) with context menu options (**Show**, **Quit**).
- **Diagnostics & Smartctl Resolution**: Multi-stage auto-detection checks custom user paths, system `PATH`, standard `Program Files` installations, and Windows Registry (`HKLM\SOFTWARE\smartmontools`). Built-in Diagnostics panel allows path browsing, re-detection, and inspecting raw outputs.
- **Rotating File Logger & Crash Hook**: Automatic rotating logging (`<data dir>/estus.log`, rotated at 1 MB) and an uncaught panic hook to capture runtime diagnostics cleanly.
- **Autostart with Windows**: Native Windows Task Scheduler integration (`schtasks /Create /RL HIGHEST /SC ONLOGON`) bypassing UAC registry blocks, starting silently to the system tray on logon.
- **Embedded SQLite Storage**: Built-in SQLite database using WAL mode (`PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;`) in the user data directory, automatically migrating schemas and pruning process writes older than 90 days.
- **Ultra-low CPU Usage**: Processes are sampled using `sysinfo` with disk-usage-only polling every 30 seconds. The GUI repaints only on data changes or at most once per second, keeping idle CPU near 0%.

---

## How the Numbers Are Measured

### 1. NVMe SMART Metrics (`smartctl`)
Estus queries `smartctl -j -a /dev/nvme0` every 10 minutes in the background without spawning console windows (`CREATE_NO_WINDOW` flag):
- **Data Units Written & Read**: Read directly from the NVMe `nvme_smart_health_information_log`. According to the official NVM Express specification, **1 Data Unit = 1,000 × 512 bytes = 512,000 bytes** (~500 KiB). Total host bytes are calculated as:
  $$\text{Host Bytes Written} = \text{data\_units\_written} \times 512{,}000$$
- **Percentage Used**: Vendor-reported drive wear indicator. `0%` indicates a brand-new drive; `100%` indicates that the drive has reached its rated endurance (TBW). Note that reaching 100% does not imply immediate drive failure, but that the manufacturer's endurance warranty threshold has been reached.
- **Available Spare**: Remaining percentage of reserve flash memory capacity available to replace degraded blocks.
- **Power-On Hours & Power Cycles**: Drive operational runtime and boot count.

### 2. Per-Process Write Tracking (`sysinfo`)
- Every 30 seconds, the background thread uses `sysinfo` with minimal refresh flags (`ProcessRefreshKind::nothing().with_disk_usage()`) to read `total_written_bytes` for each running process.
- **Delta Calculation**: Computes the delta ($\Delta = \text{current} - \text{previous}$).
- **PID Reuse Handling**: If a process exits and its PID is reused by a new process ($\text{current} < \text{previous}$), the negative delta is ignored and the baseline resets to prevent erroneous spikes.
- **Aggregation**: Write deltas are grouped and summed by process executable name so multi-threaded or multi-instance applications (such as browsers, code editors, and background sync engines) are accurately represented.

### 3. Analytics & Lifespan Projection
- **Average Daily Writes**: 7-day rolling average of daily write volume in GB:
  $$\text{Average GB/Day} = \frac{\sum_{i=1}^{N} \text{Daily Writes}_i}{N} \quad (N \le 7)$$
- **Projected Days to 100% Wear**:
  Using the drive's historical wear baseline and recent write velocity:
  $$\text{Bytes per 1\% Wear} = \frac{\text{Total Host Bytes Written}}{\text{Percentage Used}}$$
  $$\text{Remaining Wearable Bytes} = (100 - \text{Percentage Used}) \times \text{Bytes per 1\% Wear}$$
  $$\text{Projected Remaining Days} = \frac{\text{Remaining Wearable Bytes}}{\text{Average Daily Write Rate}}$$
  *(If the drive is brand new with 0% wear recorded, the projection displays "Indefinite" until wear accumulates a measurable baseline).*
- **Writes per Power-On Hour**:
  $$\text{Writes per Hour (GB)} = \frac{\text{Total Host Bytes Written}}{10^9 \times \text{Power-On Hours}}$$

---

## Architecture & Crates

The application is structured into modular Rust components:

```
src/
├── main.rs         # Application entry point, eframe initialization, tray orchestration, panic hook
├── smart.rs        # smartctl resolution, multi-drive discovery, NVMe & SATA SMART telemetry parsing
├── procs.rs        # Low-overhead sysinfo process disk polling, PID reuse delta calculation
├── db.rs           # SQLite storage (WAL mode), drives table, per-drive snapshot & delta persistence
├── analytics.rs    # Pure mathematical functions for write rates, lifespan, and thresholds
├── app_core.rs     # Background worker threads, system tray icon, notifications, JSON settings
├── ui.rs           # egui dashboard, drive selector, 30-day plot, diagnostics panel, settings
├── logger.rs       # Rotating file logger (<data dir>/estus.log) and crash formatting
└── process_util.rs # Shared silent subprocess creation with CREATE_NO_WINDOW flag
```

### Core Crates:
- **UI & Graphics**: `eframe`, `egui`, `egui_plot`
- **System Monitoring**: `sysinfo`
- **Storage**: `rusqlite` (with `"bundled"` feature)
- **Desktop Integration**: `tray-icon`, `notify-rust`, Windows Task Scheduler (`schtasks`)
- **Serialization & Config**: `serde`, `serde_json`, `directories`
- **Time & Errors**: `chrono`, `anyhow`
- **Build Manifest**: `embed-manifest` (requests administrator rights on release builds)

---

## Building from Source (For Developers)

Estus is designed to be fully portable and easy to build on any 64-bit Windows machine. You can set up and build using our automated scripts or manually configure your environment.

### 1. Quick Path (Automated Scripts)

The fastest and most reliable way to prepare your machine and compile Estus:

1. **Install All Dependencies**:
   - Double-click `setup.bat` (or run `.\setup.ps1` in PowerShell as administrator).
   - This script checks for and installs all required dependencies via Windows Package Manager (`winget`):
     - **Git**
     - **Rust** (via `rustup` with the `stable-x86_64-pc-windows-msvc` toolchain)
     - **Visual Studio C++ Build Tools** (MSVC compiler, Windows 10/11 SDK, Desktop C++ workload)
     - **smartmontools** (for `smartctl`)
2. **Build the Release Executable**:
   - Run `.\build.ps1` in PowerShell:
     ```powershell
     .\build.ps1
     ```
   - The script verifies prerequisites, detects whether Estus is running, sets up target paths, and executes `cargo build --release`.

#### Script Exit Codes
The automation scripts (`setup.ps1` and `build.ps1`) use standardized exit codes:
- `0`: Success
- `1`: General Failure
- `2`: Missing Dependency (e.g., Rust or VS C++ Build Tools not found)
- `3`: Elevation Required (script must be run as Administrator)
- `4`: App Running (an instance of `estus.exe` is running and locking the executable)

---

### 2. Manual Path

If you prefer to configure your environment manually:

1. **Install Rust**:
   - Install Rust via [rustup.rs](https://rustup.rs/) using the MSVC toolchain (`stable-x86_64-pc-windows-msvc`):
     ```powershell
     winget install --id Rustlang.Rustup
     ```
   - Verify `cargo` is available in your shell:
     ```powershell
     rustup default stable-x86_64-pc-windows-msvc
     ```
   - Ensure `%USERPROFILE%\.cargo\bin` is in your `PATH` (restart your terminal if needed).

2. **Install Visual Studio C++ Build Tools**:
   - Install Visual Studio Build Tools with the **"Desktop development with C++"** workload (including MSVC compiler and Windows SDK):
     ```powershell
     winget install Microsoft.VisualStudio.2022.BuildTools
     ```

3. **Install `smartmontools`**:
   - Install via `winget`:
     ```powershell
     winget install smartmontools.smartmontools
     ```
   - *Alternatively, download from [smartmontools.org](https://www.smartmontools.org/) and ensure `smartctl.exe` is in your `PATH`.*

4. **Build the Standalone Release Binary**:
   - Navigate to the project root directory and run:
     ```powershell
     cargo build --release
     ```

---

### 3. Strict Release-Only Build Workflow

> [!IMPORTANT]
> **Strict Release-Only Rule**: Always build, check, test, and lint using the `--release` flag:
> - `cargo build --release` (or `.\build.ps1`)
> - `cargo check --release`
> - `cargo clippy --release`
> - `cargo test --release`
>
> Debug builds are intentionally disabled (`debug = 0`, `incremental = false` in `Cargo.toml`) to prevent generating unstripped binaries, bloated symbol caches, or massive multi-gigabyte build artifacts. Never run unadorned `cargo build`, `cargo check`, `cargo clippy`, or `cargo test` without `--release`.

---

### 4. Build Output & Custom Target Directory

By default, Cargo outputs the compiled executable to the local `target` directory:
```
target\release\estus.exe
```

#### Storing Build Artifacts Externally (`ESTUS_TARGET_DIR`)
Intermediate Rust compilation artifacts can consume several gigabytes of disk space and generate significant disk write volume. If you wish to store build artifacts on a separate drive or dedicated cache directory, set the `ESTUS_TARGET_DIR` environment variable before building:

```powershell
# Point to a dedicated directory or separate drive (e.g. D:\cargo-target)
$env:ESTUS_TARGET_DIR = "D:\cargo-target"
.\build.ps1
```

Or when running Cargo directly:
```powershell
$env:CARGO_TARGET_DIR = "D:\cargo-target"
cargo build --release
```

When configured, the final executable will be located at:
```
<custom-target-dir>\release\estus.exe
```

---

### 5. Administrator Rights & Rebuilding Notes

- **Administrator Privileges Manifest**:
  Estus automatically embeds a Windows application manifest (`requireAdministrator`) into release builds. When launched, Windows displays the UAC prompt to request elevated privileges. This is necessary to communicate with physical NVMe controllers, query SMART health telemetry, and configure autostart via Windows Task Scheduler (`schtasks`).
- **File Locking During Rebuilds (`Access is denied`)**:
  Because Estus runs in the background and minimizes to the system notification tray, the binary (`estus.exe`) remains open and locked by the Windows kernel. Attempting to recompile while Estus is running will fail at the link stage with:
  ```
  error: could not copy "..." to "...": Access is denied. (os error 5)
  ```
  **Always right-click the Estus tray icon and choose "Quit" before rebuilding.**

---

### 6. What Makes This Executable Special?

- **Single Self-Contained Binary**: All dependencies (including SQLite via the bundled feature) are compiled directly into `estus.exe` (~17 MB). No external DLLs, Node.js, Electron, or web runtimes are needed.
- **Embedded Administrator Manifest**: Automatically requests `requireAdministrator` without requiring you to manually right-click "Run as administrator".
- **Zero Console Window (`windows_subsystem`)**: Compiled with native Windows GUI subsystem flags. Double-clicking `estus.exe` launches directly into the dashboard and system tray without flashing a command prompt window (`CREATE_NO_WINDOW`).

---

### 7. Running & Distributing

- You can move or copy `estus.exe` anywhere on your machine (e.g., `C:\Program Files\Estus\estus.exe` or your Desktop).
- Double-click `estus.exe` to run. It will minimize to your system tray on window close and continue background monitoring.
- Diagnostics and execution logs are stored at `%LOCALAPPDATA%\estus\Estus\data\estus.log`.

---

## Development & Verification

Always include the `--release` flag for all cargo operations:

### Run Checks
```powershell
cargo check --release
```

### Run Tests
```powershell
cargo test --release
```

### Run Clippy Linter
```powershell
cargo clippy --release --all-targets -- -D warnings
```

---

## Troubleshooting

Common issues and their solutions when setting up, building, or running Estus:

| Issue | Cause | Solution |
| :--- | :--- | :--- |
| **`cargo : The term 'cargo' is not recognized`** | Rust is not installed, or `cargo` is not in your current shell's `PATH`. | Run `setup.bat` (or `setup.ps1`), or manually add `%USERPROFILE%\.cargo\bin` to your system `PATH` and restart your terminal. |
| **`link.exe not found`** | The MSVC C++ linker or Windows SDK is missing. | Install Visual Studio C++ Build Tools and select the **"Desktop development with C++"** workload. |
| **`Access is denied (os error 5)`** | Estus is currently running, locking `estus.exe` from being overwritten. | Right-click the Estus icon in the Windows system tray and select **Quit** before running `build.ps1` or `cargo build --release`. |
| **`smartctl not detected`** | `smartctl.exe` is not installed or not in your `PATH` or registry. | Run `winget install smartmontools.smartmontools` or download the installer from [smartmontools.org](https://www.smartmontools.org/). Alternatively, open the **Diagnostics** panel inside Estus and click **Browse...** to select your `smartctl.exe` binary. |
| **`winget : The term 'winget' is not recognized`** | Windows Package Manager is not present on this Windows installation. | Install **App Installer** from the Microsoft Store, or download the latest `.msixbundle` from the official [winget-cli GitHub repository](https://github.com/microsoft/winget-cli). |
| **`running scripts is disabled on this system`** | PowerShell script execution policy restricts running `.ps1` files. | Run `setup.bat` directly (which automatically executes PowerShell with `-ExecutionPolicy Bypass`), or run `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass` in your PowerShell terminal. |

---

## Releasing

For maintainer release instructions, see [RELEASING.md](RELEASING.md).

---

## License

MIT OR Apache-2.0
