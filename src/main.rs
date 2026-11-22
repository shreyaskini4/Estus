#![windows_subsystem = "windows"]

pub mod analytics;
pub mod app_core;
pub mod autostart;
pub mod db;
pub mod logger;
pub mod process_util;
pub mod procs;
pub mod single_instance;
pub mod smart;
pub mod ui;

use std::sync::Arc;

fn main() -> eframe::Result<()> {
    // Install panic hook so uncaught panics are captured into estus.log
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let panic_msg = crate::logger::format_panic_info(panic_info);
        crate::logger::log_error(&panic_msg);
        default_hook(panic_info);
    }));

    // Single-instance guard: if another instance is running, bring it forward and exit cleanly
    let single_instance = Arc::new(crate::single_instance::acquire_or_activate());

    // Inspect CLI arguments for startup flags
    let start_minimized = std::env::args().any(|arg| arg == "--minimized");

    crate::logger::log_info(&format!(
        "Estus starting up (minimized: {})",
        start_minimized
    ));

    // Automatically sync autostart task path if the binary has been relocated
    let _ = crate::autostart::sync_task_if_path_changed();

    // 1. Load settings and create shared application state
    let settings = app_core::Settings::load();
    let state = app_core::new_app_state(settings);
    if start_minimized
        && let Ok(mut s) = state.lock() {
            s.show_window = false;
            s.is_logon_startup = true;
        }

    // 2. Start background worker threads (SMART poller, procs poller, database pruning)
    let _threads = app_core::start_background_threads(Arc::clone(&state), start_minimized);

    // 3. Initialize system tray
    let tray_bundle = app_core::init_tray().ok();

    // 4. Configure native window options
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Estus")
            .with_visible(!start_minimized)
            .with_inner_size([920.0, 740.0])
            .with_min_inner_size([780.0, 520.0]),
        ..Default::default()
    };

    let single_instance_main = Arc::clone(&single_instance);
    let res = eframe::run_native(
        "Estus",
        native_options,
        Box::new(move |cc| {
            let mut app = ui::EstusApp::new(Arc::clone(&state));

            // Spawn activation event listener thread so external launches restore/focus this instance
            let act_state = Arc::clone(&state);
            let act_ctx = cc.egui_ctx.clone();
            let act_guard = Arc::clone(&single_instance);
            std::thread::Builder::new()
                .name("estus-activation-listener".to_string())
                .spawn(move || loop {
                    if let Ok(s) = act_state.lock()
                        && s.should_quit {
                            break;
                        }
                    if act_guard.wait_for_activation(200) {
                        crate::logger::log_info("Activation signal received; unhiding and focusing window");
                        if let Ok(mut s) = act_state.lock() {
                            s.show_window = true;
                        }
                        act_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        act_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        act_ctx.request_repaint();
                    }
                })
                .expect("Failed to spawn activation listener thread");

            if let Some((tray, show_id, quit_id)) = tray_bundle {
                // Spawn tray event handler thread with egui Context so tray clicks instantly wake and show UI
                let state_clone = Arc::clone(&state);
                let ctx_clone = cc.egui_ctx.clone();
                let show_id_clone = show_id.clone();
                let quit_id_clone = quit_id.clone();

                std::thread::Builder::new()
                    .name("estus-tray-listener".to_string())
                    .spawn(move || loop {
                        let show = app_core::handle_tray_events(&state_clone, &show_id_clone, &quit_id_clone);
                        if show {
                            ctx_clone.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx_clone.send_viewport_cmd(egui::ViewportCommand::Focus);
                            ctx_clone.request_repaint();
                        }
                        if let Ok(s) = state_clone.lock()
                            && s.should_quit {
                                ctx_clone.send_viewport_cmd(egui::ViewportCommand::Close);
                                std::process::exit(0);
                            }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    })
                    .expect("Failed to spawn tray event listener thread");

                // Keep tray icon alive in the app instance
                app.set_tray(tray, show_id, quit_id);
            }

            Ok(Box::new(app))
        }),
    );

    // Keep primary instance mutex guard alive until process exit
    drop(single_instance_main);

    if let Err(ref e) = res {
        crate::logger::log_error(&format!("eframe::run_native returned error: {e}"));
    }
    res
}
