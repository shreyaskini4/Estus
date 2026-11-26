# Changelog

All notable changes to **Estus** will be documented in this file.

## [0.2.0] - 2026-11-26

### Added
- **Theme & Design System (`src/theme.rs`)**: Dark-first Fluent/Linear-inspired palette (`#0F1115` base) with Light and OS-following System theme modes, custom geometric `AppIcon` vector icons (zero emojis), and WCAG AA semantic health colors.
- **Redesigned Dashboard (`src/ui/dashboard.rs`)**: Animated 270-degree wear arc gauge, human-readable Projected Lifespan, aligned responsive stat-tile grid (`Total Host Writes`, `Writes Today` with alert progress bar, `Avg GB / Day`, `Available Spare`, `Power-On Hours`, `Writes / Hour`), and restyled 30-day write trend chart with rounded bars, threshold line, today highlight, and hover tooltip cards.
- **Collapsible Sidebar & Drive Cards (`src/ui/sidebar.rs`)**: 240px/56px left navigation rail with "All Drives" summary, individual drive cards (mini wear bar, health status dot, inline nickname editor), and `smartctl` status indicator.
- **Process Breakdown (`src/ui/processes.rs`)**: Searchable top writing processes list with horizontal proportional bars, tabular right-aligned percentages and sizes, and `Today` / `7 Days` / `30 Days` segmented filter.
- **Settings, Diagnostics & Onboarding (`src/ui/settings.rs`, `src/ui/diagnostics.rs`, `src/ui/onboarding.rs`)**: Grouped settings cards (Appearance, Daily Write Alerts, Startup & Logging, Connected Drives, Reset History), Diagnostics status card with copyable raw `smartctl` output and 24-hour intervals CSV export, onboarding/empty states, and loading skeleton shimmer.
- **32x32 Dynamic Tray Icon (`src/app_core.rs`)**: Anti-aliased tray icon with dynamic drive health status indicator.

### Fixed
- **Idle-CPU Repaint Contract**: Removed the unconditional 1-second repaint loop; UI now repaints only on background data changes, user interaction, or active animations.
- **README Documentation**: Updated SMART polling interval note to 5 minutes, updated modular architecture tree, and standardized project naming to Estus.
