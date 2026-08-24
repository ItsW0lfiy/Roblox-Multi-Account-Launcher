#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diagnostics;
mod launch;
mod local_state;
mod model;
mod platform;
mod powershell;
mod settings;

use app::LauncherApp;
use platform::AppInstanceGuard;

fn main() -> eframe::Result {
    let Some(instance_guard) = AppInstanceGuard::acquire("Wolfy_RobloxMultiAccountLauncher") else {
        platform::activate_existing_window("Roblox Multi-Account Launcher");
        return Ok(());
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Roblox Multi-Account Launcher")
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Roblox Multi-Account Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(LauncherApp::new(cc, instance_guard)))),
    )
}
