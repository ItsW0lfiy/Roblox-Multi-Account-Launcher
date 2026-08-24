#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod diagnostics;
mod history;
mod hotkeys;
mod instance_paths;
mod launch;
mod links;
mod local_state;
mod model;
mod platform;
mod powershell;
mod settings;
mod support;
mod updater;

use app::LauncherApp;
use platform::AppInstanceGuard;

fn main() -> eframe::Result {
    match updater::parse_internal_mode(std::env::args_os()) {
        Ok(Some(mode)) => {
            if updater::run_internal_mode(mode) {
                return Ok(());
            }
        }
        Ok(None) => {}
        Err(message) => {
            updater::record_helper_error(&message);
            return Ok(());
        }
    }
    let smoke_test = std::env::args().any(|arg| arg.eq_ignore_ascii_case("--smoke-test"));
    let instance_name = if smoke_test {
        format!(
            "Wolfy_RobloxMultiAccountLauncher_Smoke_{}",
            std::process::id()
        )
    } else {
        "Wolfy_RobloxMultiAccountLauncher".into()
    };
    let Some(instance_guard) = AppInstanceGuard::acquire(&instance_name) else {
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
