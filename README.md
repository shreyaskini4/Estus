# Estus

A lightweight, native Windows desktop application written in Rust that monitors and analyzes what is wearing out your NVMe SSD.

Designed with a focus on efficiency: **single small executable**, **negligible CPU and RAM overhead**, **zero web technology** (no Electron, no WebView, no Chromium).

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
- **Autostart with Windows**: Native Windows startup registry integration via the `auto-launch` crate.
- **Embedded SQLite Storage**: Built-in SQLite database using WAL mode (`PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;`) in the user data directory, automatically migrating schemas and pruning process writes older than 90 days.
- **Ultra-low CPU Usage**: Processes are sampled using `sysinfo` with disk-usage-only polling every 30 seconds. The GUI repaints only on data changes or at most once per second, keeping idle CPU near 0%.

---

## Installation & Setup

### 1. Install `smartmontools`
SSD Watch uses `smartctl` to read low-level NVMe SMART health metrics. Install it via Windows Package Manager (`winget`):

```powershell
winget install smartmontools
```

*Alternatively, download the installer directly from [smartmontools.org](https://www.smartmontools.org/) and ensure `smartctl.exe` is in your system `PATH`.*

### 2. Run as Administrator
Accessing raw NVMe hardware SMART logs on Windows requires elevated administrator privileges:
- The release binary includes an **embedded Windows application manifest** that automatically triggers the Windows UAC (User Account Control) prompt requesting administrator rights (`requireAdministrator`).
- If launching from a terminal or custom shortcut, ensure it is executed from an elevated shell or with "Run as Administrator".

---

## How the Numbers Are Measured

### 1. NVMe SMART Metrics (`smartctl`)
SSD Watch queries `smartctl -j -a /dev/nvme0` every 10 minutes in the background without spawning console windows (`CREATE_NO_WINDOW` flag):
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
- **Desktop Integration**: `tray-icon`, `notify-rust`, `auto-launch`
- **Serialization & Config**: `serde`, `serde_json`, `directories`
- **Time & Errors**: `chrono`, `anyhow`
- **Build Manifest**: `embed-manifest` (requests administrator rights on release builds)

---

## Creating the Executable (.exe)

### 1. Prerequisites
- **Rust Toolchain**: Install Rust via [rustup.rs](https://rustup.rs/) or PowerShell:
  ```powershell
  winget install --id Rustlang.Rustup
  ```
  *(Restart your terminal after installation so `cargo` is in your `PATH`)*
- **C++ Build Tools**: Visual Studio C++ Build Tools (MSVC toolchain).
- **smartmontools**: Required at runtime to query drive telemetry:
  ```powershell
  winget install smartmontools
  ```

---

### 2. Build the Standalone Release Binary
Navigate to the project root directory and run:

```powershell
cargo build --release
```

Once compilation completes, your standalone executable is ready at:
```
target\release\estus.exe
```

---

### 3. What Makes This Executable Special?
- **Single Self-Contained Binary**: All dependencies (including SQLite via the bundled feature) are compiled directly into `estus.exe` (~17 MB). No external DLLs, Node.js, Electron, or web runtimes are needed.
- **Embedded Administrator Manifest**: During release builds, `build.rs` automatically embeds a Windows application manifest requesting `requireAdministrator`. When you launch `estus.exe`, Windows automatically prompts for UAC elevation without requiring you to right-click "Run as administrator".
- **Zero Console Window (`windows_subsystem`)**: Compiled with native Windows GUI subsystem flags. Double-clicking `estus.exe` launches the application directly into the system tray and dashboard without flashing or leaving a command prompt window open.

---

### 4. Running & Distributing
- You can move or copy `estus.exe` anywhere on your machine (e.g., `C:\Program Files\Estus\estus.exe` or your Desktop).
- Double-click `estus.exe` to run. It will minimize to your system tray on window close and continue background monitoring.
- Diagnostics and execution logs are stored at `%LOCALAPPDATA%\estus\Estus\data\estus.log`.

---

## Development & Verification

### Run Tests
```powershell
cargo test
```

### Run Clippy Linter
```powershell
cargo clippy --all-targets -- -D warnings
```

---

## License

MIT OR Apache-2.0
