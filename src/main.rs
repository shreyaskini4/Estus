#![windows_subsystem = "windows"]

pub mod analytics;
pub mod app_core;
pub mod db;
pub mod logger;
pub mod process_util;
pub mod procs;
pub mod smart;
pub mod ui;

use std::sync::Arc;

fn main() -> eframe::Result<()> {
    // Install panic hook so uncaught panics are captured into estus.log
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let panic_msg = crate::logger::format_panic_info(panic_info);
        crate::logger::log(&panic_msg);
        default_hook(panic_info);
    }));

    crate::logger::log("Estus starting up");

    // 1. Load settings and create shared application state
    let settings = app_core::Settings::load();
    let state = app_core::new_app_state(settings);

    // 2. Start background worker threads (SMART poller, procs poller, database pruning)
    let _threads = app_core::start_background_threads(Arc::clone(&state));

    // 3. Initialize system tray
    let tray_bundle = app_core::init_tray().ok();

    // 4. Configure native window options
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Estus")
            .with_inner_size([920.0, 740.0])
            .with_min_inner_size([780.0, 520.0]),
        ..Default::default()
    };

    let res = eframe::run_native(
        "Estus",
        native_options,
        Box::new(move |cc| {
            let mut app = ui::EstusApp::new(Arc::clone(&state));

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

    if let Err(ref e) = res {
        crate::logger::log(&format!("eframe::run_native returned error: {e}"));
    }
    res
}
