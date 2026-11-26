---
name: egui-modern-ui
description: Design tokens, layout/spacing rules, vector icon policy, idle-CPU repaint contract, and Rust/clippy hygiene for the Estus egui UI redesign.
---

# Estus `egui` Modern UI Guidelines

All agents implementing or modifying Estus UI modules MUST follow these rules.

## 1. Design Tokens (Always Use `crate::theme::Theme`)
- **Never hardcode raw `Color32::from_rgb(...)`** in view files (`dashboard.rs`, `sidebar.rs`, `processes.rs`, `settings.rs`, `diagnostics.rs`, `onboarding.rs`). Always read colors from `theme: &Theme`.
- **Dark-first + Light + System**: Every component must look crisp in both Dark (`#0F1115` base, `#161920` surface, `#1C2029` elevated, `#22D3EE` accent) and Light (`#F8FAFC` base, `#FFFFFF` surface, `#F1F5F9` elevated, `#0E7490` / `#0891B2` accent).
- **Semantic health colors**:
  - `< 50%` wear: `theme.health_green`
  - `50%..80%` wear: `theme.health_amber`
  - `>= 80%` wear or threshold exceeded / error: `theme.health_red`
- **Spacing scale**: Only use `4.0`, `8.0`, `12.0`, `16.0`, `24.0`, `32.0` px (`theme::SPACE_XS` .. `theme::SPACE_2XL`).
- **Radii scale**: Only use `4.0`, `6.0`, `10.0`, `12.0`, `999.0` px (`theme::RADIUS_XS` .. `theme::RADIUS_PILL`).
- **Typography**:
  - `Hero` (`36.0`), `Metric` (`22.0`), `Heading` (`16.0`), `Body` (`13.0`), `Label` (`11.0`), `Mono` (`12.0`).
  - Use monospace / tabular alignment for numbers in tables, byte counts, and percentages so columns never jitter.

## 2. Icon Policy (Zero Emojis)
- **NEVER use Unicode emojis** (`🔄`, `🌐`, `⚡`, `💾`, `📁`, `🛠`, `⚙`, `⚠️`, `✓`, `✗`, etc.) anywhere in UI strings, labels, or buttons.
- Always use `theme::AppIcon` rendered via `theme::paint_icon(...)` or `theme::icon_button(...)` / `theme::pill_button(...)`.

## 3. Layout & Responsiveness
- **Use shared primitives** from `src/theme.rs`: `card()`, `stat_tile()`, `pill_button()`, `segmented_control()`, `section_header()`, `badge()`, `icon_button()`, `toggle_switch()`.
- **Responsive grids**: Inspect `ui.available_width()` and adapt column counts cleanly from `900×600` up to 4K and across `100%/125%/150%` DPI scaling. Avoid fixed pixel widths that overflow narrow panels.
- **No `ui.columns(N)` for unequal/wrapping cards** without checking `available_width()`; prefer explicit `egui::Grid` or calculated card widths with `ui.allocate_ui_with_layout`.

## 4. Idle CPU & Repaint Contract
- **NEVER** add unconditional `ctx.request_repaint()` or `ctx.request_repaint_after(Duration::from_secs(1))` in view code.
- Only call `ui.ctx().request_repaint()` while an active animation (e.g. gauge ease-in, skeleton shimmer, toggle transition) is still in progress (`progress < 1.0`). Once settled, stop calling `request_repaint()`.
- Background data updates wake the UI via `SharedState`'s `egui::Context` handle.

## 5. Rust & Clippy Hygiene
- Always run Cargo commands with **`--release`**:
  - `cargo check --release`
  - `cargo clippy --release --all-targets -- -D warnings`
  - `cargo test --release`
- Keep `src/ui/*` functions small, single-purpose, and free of `#[allow(clippy::too_many_arguments)]` (pass `&Theme` and `&UiSnapshot` instead of 15 loose arguments).
- Preserve all existing unit tests and add unit tests for any new pure formatting/state logic.
- Only modify files assigned to your agent in `docs/ui-redesign-spec.md` Section 7.
