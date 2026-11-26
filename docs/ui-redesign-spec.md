# Estus UI Redesign Specification

## 1. Current Codebase & UI Audit

### 1.1 Architecture & File Layout
- **`src/main.rs`**: Entry point, panic hook, Windows single-instance guard (`single_instance.rs`), `--minimized` logon flag handling, background thread startup, `tray-icon` orchestration, and `eframe::run_native` with initial window size `920×740` (min `780×520`).
- **`src/ui.rs` (2,097 lines)**: Monolithic UI file containing `EstusApp` (`SsdWatchApp`), formatting helpers (`format_bytes`, `format_with_commas`, `format_projected_lifespan`, `format_percentage`, `format_intervals_csv`), `eframe::App` implementation, all panel renderers, and unit tests.
- **`src/app_core.rs`**: `Settings` (JSON persistence via `directories`), `SharedState` (`Arc<Mutex<SharedState>>`), `SMART_POLL_INTERVAL_SECS = 300` (5 minutes), 30-second process poller, Windows toast notifications (`notify-rust`), and `init_tray()` / `handle_tray_events()`.
- **`src/smart.rs`**, **`src/procs.rs`**, **`src/db.rs`**, **`src/analytics.rs`**, **`src/autostart.rs`**, **`src/logger.rs`**, **`src/process_util.rs`**, **`src/single_instance.rs`**: Backend/system modules with clean separation and comprehensive unit tests.

### 1.2 Current UI Structure, Controls & Styling
- **Single-page vertical scroll**: `egui::CentralPanel` with a single `egui::ScrollArea::vertical()` stacking 9 sections top-to-bottom:
  1. Top header (`Estus`, `SSD Health & Write Monitor`, `🔄 Refresh Now`).
  2. Drive selector bar (`DRIVES:`, `🌐 [ All drives ]`, per-drive `selectable_label` buttons with `⚡`/`💾`, `🔄 Rescan`).
  3. Status bar (`smartctl:` path, version, `Device:`, `📁 Browse...`, optional `Last raw error:`).
  4. Conditional error banners (`initialization_error`, `smart_error` with `⚠️`).
  5. 4-column `ui.columns(4, ...)` metric cards (`PERCENTAGE USED`, `TOTAL HOST WRITES`, `WRITES TODAY`, `PROJECTED LIFESPAN`) using `egui::ProgressBar` for wear.
  6. 30-day `egui_plot::Plot` stacked bar chart (`Active writes` blue `(59, 130, 246)` + `Offline gap` amber `(245, 158, 11)`).
  7. Top writing processes table (`Today` / `7 Days` / `30 Days` `selectable_label`s, striped `egui::Grid`).
  8. Collapsible `⚙ Settings & Actions` (`default_open(false)`).
  9. Collapsible `🛠 Diagnostics` (`default_open(true)`).
- **Hardcoded dark-only visuals**: `ui.ctx().set_visuals(egui::Visuals::dark())` called every frame. No Light or System theme support. Ad-hoc inline `Color32::from_rgb(...)` values (`20/22/28`, `22/24/30`, `28/30/36`, `45/48/58`, etc.).
- **Emoji reliance**: Uses `🔄`, `🌐`, `⚡`, `💾`, `📁`, `🛠`, `⚙`, `⚠️`, `✓`, `✗` directly in `RichText`. In `egui`'s default embedded font on Windows, many of these render as monochrome boxes/tofu or mismatched glyphs.
- **Tray icon (`app_core.rs`)**: Hardcoded 16×16 pixel rectangle (`[41, 128, 185]` inside `[220, 220, 220]` with `[40, 40, 40]` 1px border). Does not reflect drive health or support 32×32 high-DPI.

### 1.3 Repaint Behavior & Idle CPU
- `src/ui.rs` line 254 calls `ui.ctx().request_repaint_after(std::time::Duration::from_secs(1))` unconditionally every frame, waking the UI thread once per second even when idle and minimized.
- Background pollers set `s.data_changed = true` in `SharedState`, but do not call `egui::Context::request_repaint()` directly.

### 1.4 UX Pain Points & Documentation Discrepancies
- **Information overload**: All sections live in one long vertical scroll; `Diagnostics` defaults to open at the bottom, pushing primary monitoring content and settings into a single cluttered document.
- **Poor narrow-width responsiveness**: `ui.columns(4)` squishes metric cards at 780–900px widths; drive tabs wrap unevenly.
- **Flat data presentation**: Wear % is a bare number with a 4px bar; process rows lack proportional visual bars, search/filter, and tabular numeral alignment.
- **README.md discrepancies**:
  - Line 70 states `smartctl -j -a` runs every 10 minutes, whereas `app_core.rs` sets `SMART_POLL_INTERVAL_SECS = 300` (5 minutes).
  - Architecture tree lists monolithic `src/ui.rs` rather than modular `src/theme.rs` and `src/ui/*`.

---

## 2. Overview & Aesthetic Direction

Estus will adopt a **dark-first, Fluent / Linear / Raycast-inspired Windows 11 utility** aesthetic:
- **Focused, high-craft workspace**: A persistent collapsible left sidebar for drive selection and view navigation, paired with a clean single-view main canvas (`Dashboard`, `Processes`, `Settings`, `Diagnostics`).
- **Crisp geometry & restraint**: Deep neutral surfaces (`#0F1115` base in Dark), 1px hairline borders, generous whitespace, and a single crisp Teal/Cyan accent (`#22D3EE` dark / `#0891B2` light) reserved for active states, primary actions, and data highlights.
- **Zero emojis**: Every UI icon is rendered via custom `egui::Painter` vector primitives (`AppIcon`) for pixel-sharp rendering at 100%, 125%, and 150%+ DPI.
- **Three theme modes**: `Dark` (default), `Light`, and `System` (follows Windows OS preference via `egui::Theme::System` / `ctx.system_theme()`).

---

## 3. Design Tokens

### 3.1 Color Palette (WCAG AA Verified)

#### Dark Theme (Default)
| Token | Hex | RGB | Usage & Contrast |
| :--- | :--- | :--- | :--- |
| `bg_base` | `#0F1115` | `(15, 17, 21)` | Window & main canvas background |
| `bg_sidebar` | `#12151B` | `(18, 21, 27)` | Left navigation & drive rail |
| `bg_surface` | `#161920` | `(22, 25, 32)` | Primary cards, panels |
| `bg_elevated` | `#1C2029` | `(28, 32, 41)` | Hover states, stat tiles, tooltips, inputs |
| `border_subtle` | `#252A36` | `(37, 42, 54)` | Card & divider 1px hairlines |
| `border_strong` | `#343B4C` | `(52, 59, 76)` | Focused inputs, active card outlines |
| `text_primary` | `#F1F5F9` | `(241, 245, 249)` | Primary headings & values (**14.9:1** on `bg_elevated`) |
| `text_secondary` | `#94A3B8` | `(148, 163, 184)` | Subtitles, descriptions (**6.2:1** on `bg_elevated`) |
| `text_muted` | `#7C8B9E` | `(124, 139, 158)` | 11px labels, table headers (**4.6:1** on `bg_elevated`, AA) |
| `accent` | `#22D3EE` | `(34, 211, 238)` | Active nav, primary CTA, chart active bars (**9.7:1** on `bg_elevated`) |
| `accent_muted` | `#164E63` | `(22, 78, 99)` | Active pill tint / selection fill |
| `accent_on_fill` | `#081216` | `(8, 18, 22)` | Text/icon on solid `accent` button (**12.1:1**) |
| `health_green` | `#34D399` | `(52, 211, 153)` | Wear < 50%, healthy status (**9.9:1** on `bg_surface`) |
| `health_amber` | `#FBBF24` | `(251, 191, 36)` | Wear 50–80%, offline gap bars (**10.6:1** on `bg_surface`) |
| `health_red` | `#F87171` | `(248, 113, 113)` | Wear > 80%, threshold exceeded, errors (**6.6:1** on `bg_surface`) |

#### Light Theme
| Token | Hex | RGB | Usage & Contrast |
| :--- | :--- | :--- | :--- |
| `bg_base` | `#F8FAFC` | `(248, 250, 252)` | Window & main canvas background |
| `bg_sidebar` | `#F1F5F9` | `(241, 245, 249)` | Left navigation & drive rail |
| `bg_surface` | `#FFFFFF` | `(255, 255, 255)` | Primary cards, panels |
| `bg_elevated` | `#F1F5F9` | `(241, 245, 249)` | Stat tiles, hover states, inputs |
| `border_subtle` | `#E2E8F0` | `(226, 232, 240)` | Card & divider 1px hairlines |
| `border_strong` | `#CBD5E1` | `(203, 213, 225)` | Focused inputs, active card outlines |
| `text_primary` | `#0F172A` | `(15, 23, 42)` | Primary headings & values (**16.3:1** on `bg_elevated`) |
| `text_secondary` | `#475569` | `(71, 85, 105)` | Subtitles, descriptions (**6.8:1** on `bg_elevated`) |
| `text_muted` | `#5B6B82` | `(91, 107, 130)` | 11px labels, table headers (**4.8:1** on `bg_elevated`, AA) |
| `accent` | `#0891B2` | `(8, 145, 178)` | Graphic highlights, bars, borders |
| `accent_text` | `#0E7490` | `(14, 116, 144)` | Accent text & solid button fill (**5.0:1** with white text / on white) |
| `accent_muted` | `#ECFEFF` | `(236, 254, 255)` | Active pill tint / selection fill |
| `accent_on_fill` | `#FFFFFF` | `(255, 255, 255)` | Text/icon on solid `accent_text` button (**5.0:1**) |
| `health_green` | `#047857` | `(4, 120, 87)` | Wear < 50%, healthy status (**5.4:1** on `bg_surface`) |
| `health_amber` | `#B45309` | `(180, 83, 9)` | Wear 50–80%, offline gap bars (**5.3:1** on `bg_surface`) |
| `health_red` | `#DC2626` | `(220, 38, 38)` | Wear > 80%, threshold exceeded, errors (**4.8:1** on `bg_surface`) |

### 3.2 Spacing & Radii Scales
- **Spacing scale**: `4.0`, `8.0`, `12.0`, `16.0`, `24.0`, `32.0` px (`SPACE_XS`, `SPACE_SM`, `SPACE_MD`, `SPACE_LG`, `SPACE_XL`, `SPACE_2XL`).
- **Radii scale**: `4.0` (inputs/micro-badges), `6.0` (buttons/pills), `10.0` (stat tiles/sub-cards), `12.0` (primary cards), `999.0` (full pills/toggles/dots).

### 3.3 Typography & Font Strategy
- **Scale**:
  - `Hero` (`36.0px`, semibold): Gauge center percentage / hero metric.
  - `Metric` (`22.0px`, semibold): Stat tile values.
  - `Heading` (`16.0px`, semibold): View titles & section headers.
  - `Body` (`13.0px`, regular): Standard labels, descriptions, table cells.
  - `Label` (`11.0px`, medium/semibold): Micro-labels, uppercase section kickers, badges.
  - `Mono` (`12.0px`, monospace): Device paths, serials, raw smartctl output, timestamps.
- **Tabular numerals**: Numeric metrics, percentages, byte counts, and table columns must align cleanly using monospace or tabular-figure-compatible formatting (`egui::FontFamily::Monospace` for numeric columns / `RichText::monospace` where needed, or system `Segoe UI Variable` / `Consolas` fallback).
- **Font Strategy**: Zero multi-megabyte bundled fonts. Load Windows system fonts at startup if present (`C:\Windows\Fonts\SegUIVar.ttf` or `segoeui.ttf` for Proportional, `C:\Windows\Fonts\consola.ttf` for Monospace, keeping total loaded font data minimal or falling back cleanly to `egui`'s built-in fonts when running in headless/test environments).

---

## 4. Layout Architecture

```
+-----------------------------------------------------------------------+
|  SIDEBAR (240px / 56px)  |  MAIN CONTENT AREA                         |
|  [Estus]      [Collapse] |  [View Header + Quick Actions]             |
|                          |                                            |
|  DRIVES                  |  +--------------------------------------+  |
|  [All Drives]            |  | Active View:                         |  |
|  [Drive 1: OS SSD  3%]   |  |  - Dashboard                         |  |
|  [Drive 2: Games   1%]   |  |  - Processes                         |  |
|                          |  |  - Settings                          |  |
|  NAVIGATION              |  |  - Diagnostics                       |  |
|  > Dashboard             |  +--------------------------------------+  |
|    Processes             |                                            |
|    Settings              |                                            |
|    Diagnostics           |                                            |
|                          |                                            |
|  [smartctl status dot]   |                                            |
+-----------------------------------------------------------------------+
```

- **Collapsible Left Sidebar**:
  - Expanded: `240.0px` width; Collapsed: `56.0px` width (icon-only nav + compact drive indicators with hover tooltips).
  - Top: Brand mark (custom Estus flame/drive vector) + title + collapse toggle `icon_button`.
  - **Drives Section**: `"All Drives"` summary card at top (shows total capacity / max wear), followed by individual drive cards displaying nickname (or model), truncated model/serial, capacity, mini wear progress bar, semantic health status dot, and an inline rename pencil `icon_button` (switches to inline `TextEdit` on click, commits on Enter or blur).
  - **Main Navigation**: `Dashboard`, `Processes`, `Settings`, `Diagnostics` with active pill highlight and custom `AppIcon`.
  - **Bottom Footer**: Compact `smartctl` health indicator pill (clickable -> jumps to `Diagnostics`) + theme quick-toggle or version tag.
- **Responsive Main Content Area**:
  - Window defaults to `1120×740`, minimum `900×600`, scaling cleanly up to 4K and across `100%`, `125%`, `150%` Windows DPI scaling.
  - Uses `ui.available_width()` breakpoints (`< 720px` narrow, `720–1050px` medium, `> 1050px` wide) to adapt grid columns (e.g., 2×3 vs 3×2 stat tiles, stacked vs side-by-side hero gauge + stat tiles).
  - Each view has its own `ScrollArea::vertical()` when content exceeds viewport height.

---

## 5. Component Inventory & Module Specifications

### 5.1 Theme & Design System (`src/theme.rs`)
- `pub enum ThemeMode { Dark, Light, System }` (`Serialize, Deserialize, Default` = `Dark`).
- `pub struct Theme`: Holds all resolved `Color32` tokens for the active mode, plus spacing/radii constants.
- `pub enum AppIcon`:
  - `DriveAll`, `DriveNvme`, `DriveSata`, `Dashboard`, `Processes`, `Settings`, `Diagnostics`, `Refresh`, `Rescan`, `Folder`, `Search`, `Pencil`, `Check`, `Close`, `Warning`, `ChevronLeft`, `ChevronRight`, `Export`, `Sun`, `Moon`, `Monitor`, `Flame`.
  - `pub fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: AppIcon, color: Color32)`: Crisp geometric vector drawing (lines, arcs, rects, circles) with stroke width scaled to rect size. **Zero emojis.**
- Reusable widget primitives:
  - `pub fn card(ui: &mut egui::Ui, theme: &Theme, add_contents: impl FnOnce(&mut egui::Ui))`
  - `pub fn stat_tile(ui: &mut egui::Ui, theme: &Theme, label: &str, value: &str, subtext: &str, accent_color: Color32, progress: Option<f32>)`
  - `pub fn pill_button(ui: &mut egui::Ui, theme: &Theme, label: &str, icon: Option<AppIcon>, primary: bool) -> egui::Response`
  - `pub fn segmented_control<T: Copy + PartialEq>(ui: &mut egui::Ui, theme: &Theme, id: &str, current: &mut T, options: &[(T, &str)]) -> bool`
  - `pub fn section_header(ui: &mut egui::Ui, theme: &Theme, title: &str, subtitle: Option<&str>)`
  - `pub fn badge(ui: &mut egui::Ui, theme: &Theme, text: &str, color: Color32)`
  - `pub fn icon_button(ui: &mut egui::Ui, theme: &Theme, icon: AppIcon, tooltip: &str) -> egui::Response`
  - `pub fn toggle_switch(ui: &mut egui::Ui, theme: &Theme, on: &mut bool) -> egui::Response`
  - `pub fn configure_fonts(ctx: &egui::Context)` & `pub fn apply_theme(ctx: &egui::Context, mode: ThemeMode) -> Theme`

### 5.2 Dashboard View (`src/ui/dashboard.rs`)
- **Top Hero Row** (responsive 2-column on medium/wide, stacked on narrow):
  - **Custom Painter Arc/Circular Wear Gauge**:
    - 270° arc track (`border_subtle`) + filled wear arc (`health_green` < 50%, `health_amber` 50–80%, `health_red` >= 80%).
    - Animated ease-in on view load or drive switch (`0.0 -> target_pct` over `450ms` cubic ease-out). Requests repaint ONLY while animation progress `< 1.0`.
    - Center hero text (`36px` bold): e.g. `3%` (`Used`), subtitle `97% remaining` (or `max` across All Drives).
  - **6 Stat Tiles** (3×2 grid on wide/medium, 2×3 on narrow):
    1. `TOTAL HOST WRITES` (e.g. `14.2 TB`, subtext `Read: 11.8 TB` or drive count)
    2. `WRITES TODAY` (e.g. `18.4 GB`, subtext `Active / Offline` or remaining budget, colored red if > threshold)
    3. `PROJECTED LIFESPAN` (readable: `~6.2 years` / `Indefinite` / `> 30 years`, subtext `Rate: X.X GB/day`)
    4. `AVAILABLE SPARE` (e.g. `100%`, subtext `Reserve blocks` / `Avg across drives`)
    5. `POWER-ON HOURS` (e.g. `1,420 h`, subtext `~59 days active`)
    6. `POWER CYCLES` (e.g. `312`, subtext `Avg X.X GB/hr`)
- **Restyled 30-Day Daily Writes Chart**:
  - Custom `egui::Painter` bar chart (or styled canvas) featuring:
    - Rounded top corners (`4px`) on bars.
    - Soft horizontal grid lines (`border_subtle` at 40% alpha) with clean Y-axis GB labels.
    - Stacked segments: `Active writes` (`accent`) + `Offline gap` (`health_amber`).
    - **Today** bar highlighted with subtle border/glow or brighter fill.
    - Dashed horizontal **Daily Alert Threshold** line (`health_red` at 70% alpha) with inline label.
    - **Hover tooltip card** (`bg_elevated`, `border_strong`, `10px` radius): shows exact date, Active GB, Offline gap GB, and Total GB.

### 5.3 Sidebar & Processes (`src/ui/sidebar.rs`, `src/ui/processes.rs`)
- **Sidebar (`src/ui/sidebar.rs`)**:
  - Collapsible rail (`240px` / `56px`).
  - `"All Drives"` card at top + scrollable list of `DriveInfo` cards with:
    - Custom `AppIcon::DriveNvme` / `AppIcon::DriveSata`.
    - Nickname (or model), capacity (`format_bytes`), mini 3px wear progress bar, semantic health dot (`green`/`amber`/`red`/`muted` if disabled).
    - Inline rename pencil `icon_button` -> inline `TextEdit` that saves nickname directly to `Settings` + `db::update_drive_settings` on Enter/commit.
  - 4-item navigation (`Dashboard`, `Processes`, `Settings`, `Diagnostics`) + `Rescan` / `Refresh` quick actions.
- **Processes (`src/ui/processes.rs`)**:
  - Header with `"Top Writing Processes"` + `"System-wide"` badge + `segmented_control` (`Today`, `7 Days`, `30 Days`).
  - Search/filter input box (`AppIcon::Search` + `TextEdit` with clear button).
  - Process list rows: Rank (`#1`), process name (monospace/body), **horizontal proportional bar** (`accent` fill proportional to top writer or total), percentage share, and formatted bytes (`tabular` alignment).
  - Clean empty state when no process deltas match or none recorded yet.

### 5.4 Settings, Diagnostics & Empty States (`src/ui/settings.rs`, `src/ui/diagnostics.rs`, `src/ui/onboarding.rs`)
- **Settings (`src/ui/settings.rs`)**:
  - Grouped cards:
    - **Appearance**: `ThemeMode` segmented control (`Dark`, `Light`, `System`).
    - **Alerts**: Daily Alert Threshold slider + numeric readout (`1–1000 GB`).
    - **Startup & Logging**: `toggle_switch` for `Start with Windows` + Task Scheduler status `badge` + `Check` button; `toggle_switch` for `Verbose logging`.
    - **Connected Drives**: Per-drive `toggle_switch` (Enabled), inline nickname editor, protocol/capacity/serial metadata.
    - **Data Management**: `Save Settings` primary `pill_button` + `Reset History` destructive action with inline confirmation card.
- **Diagnostics (`src/ui/diagnostics.rs`)**:
  - Status `badge` (`Detected` green / `Not Found` red / `Error` amber), resolved `smartctl` path, version, active device, `Browse...` and `Re-detect` buttons.
  - Collapsible `Raw smartctl Output` card with monospace viewer.
  - `Intervals (Last 24 Hours)` card with `Export CSV` `pill_button` and clean table (`Local Time`, `Raw Data Units`, `Delta (GB)`, `Attribution` badge).
- **Onboarding, Empty & Skeleton States (`src/ui/onboarding.rs`)**:
  - Actionable onboarding card when `smartctl` is missing (`winget install smartmontools` copyable/clear command + `Browse for smartctl.exe` button).
  - Permission-denied / initialization error cards using `AppIcon::Warning` (no emoji).
  - Skeleton shimmer cards during initial startup poll before first snapshot arrives (uses bounded animation timer, stops once data arrives).

### 5.5 System Tray Icon & README Updates (`src/app_core.rs`, `README.md`)
- **Tray Icon**: Generate crisp 32×32 (and 16×16) anti-aliased RGBA icon (rounded dark/neutral badge with SSD/flame silhouette and a semantic health status ring/dot: Green < 50%, Amber 50–80%, Red >= 80%, Neutral if unavailable). Update tray icon when drive health changes.
- **README.md**: Fix polling interval documentation (`10 minutes` -> `5 minutes` matching `SMART_POLL_INTERVAL_SECS = 300`), and update Architecture module list (`src/theme.rs`, `src/ui/`).

---

## 6. Idle CPU & Repaint Contract

To guarantee **near-0% idle CPU** when minimized or resting:
1. **Remove** the unconditional `ui.ctx().request_repaint_after(Duration::from_secs(1))` from `eframe::App::ui()`.
2. **Wake on data change**: Store `egui_ctx: Option<egui::Context>` (or pass to pollers) in `SharedState` so `poll_smart_cycle`, the 30s process poller, tray events, and activation listener call `ctx.request_repaint()` when `data_changed` is set.
3. **Wake on user interaction**: `egui` automatically repaints on mouse/keyboard/window events.
4. **Animate only while active**:
   - Track animation state on `EstusApp` / view state (e.g., `gauge_anim_start: Option<Instant>`, `shimmer_active: bool`, `toggle_anim_active: bool`).
   - Only call `ui.ctx().request_repaint()` during frames where an animation is actively progressing (`elapsed < duration`). Once settled, stop requesting repaints.
5. **Optional slow clock tick**: Only request `request_repaint_after(Duration::from_secs(60))` if a relative timestamp ("Updated X min ago") is visible and the window is open (`show_window == true`). Never request 1-second repaints while hidden in tray.

---

## 7. File-Ownership Map & Module Boundaries

| Agent | Owned Files | Key Public Interfaces / Responsibilities |
| :--- | :--- | :--- |
| **Agent 1** | `src/theme.rs`, `src/ui/mod.rs` (replaces `src/ui.rs`), `src/main.rs`, `src/app_core.rs` (`Settings::theme_mode`, `egui_ctx` wakeup) | Defines `ThemeMode`, `Theme`, `AppIcon`, all widget helpers, `NavTab`, `EstusApp` / `SsdWatchApp`, `UiSnapshot`, repaint contract, and stub modules so Agents 2–4 compile cleanly in parallel. |
| **Agent 2** | `src/ui/dashboard.rs` | `pub fn render(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot)` — Animated wear arc gauge, 6 stat tiles, restyled 30-day chart with hover tooltip & threshold line. |
| **Agent 3** | `src/ui/sidebar.rs`, `src/ui/processes.rs` | `sidebar::render(ui, theme, app, snap)` — Collapsible 240/56px sidebar, drive cards, inline rename, nav. `processes::render(ui, theme, app, snap)` — Searchable process table with proportional bars & segmented filter. |
| **Agent 4** | `src/ui/settings.rs`, `src/ui/diagnostics.rs`, `src/ui/onboarding.rs` | `settings::render(ui, theme, app, snap)`, `diagnostics::render(ui, theme, app, snap)`, `onboarding::render_banner(...)` / `render_skeleton(...)` / `render_empty_state(...)`. |
| **Agent 5** | `src/app_core.rs` (tray icon RGBA generator + dynamic health update), `README.md` | Crisp 32×32/16×16 health-colored tray icon, README fixes, full `cargo check/clippy/test/build --release` verification. |

### Shared UI Snapshot Struct (`src/ui/mod.rs`)
To keep `sidebar`, `dashboard`, `processes`, `settings`, and `diagnostics` decoupled from `SharedState` locking:
```rust
pub struct UiSnapshot {
    pub detected_drives: Vec<DriveInfo>,
    pub selected_drive_id: Option<String>,
    pub selected_drive: Option<DriveInfo>,
    pub selected_snapshot: Option<SmartSnapshot>,
    pub selected_daily_writes: Vec<crate::db::DailyWrites>,
    pub selected_intervals_24h: Vec<crate::db::SnapshotInterval>,
    pub per_drive_snapshots: std::collections::HashMap<String, SmartSnapshot>,
    pub smart_error: Option<SmartError>,
    pub top_today: Vec<(String, u64)>,
    pub top_7d: Vec<(String, u64)>,
    pub top_30d: Vec<(String, u64)>,
    pub settings: Settings,
    pub resolved_smartctl_path: Option<String>,
    pub smartctl_version: Option<String>,
    pub active_device: Option<String>,
    pub last_raw_error: Option<String>,
    pub last_raw_output: Option<String>,
    pub initialization_error: Option<String>,
}
```
All formatting helpers (`format_bytes`, `format_with_commas`, `format_projected_lifespan`, `format_percentage`, `format_intervals_csv`) remain `pub` in `src/ui/mod.rs` (re-exported via `crate::ui::*`) so existing unit tests and callers continue to work seamlessly.
