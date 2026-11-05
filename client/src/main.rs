// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod app;
mod config;
mod dialogs;
mod fmt;
mod workspace;

slint::include_modules!();

use std::time::Duration;

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ctx = app::Ctx::new(&ui);

    // Pick up other people's check-outs and new versions without clicking "Atualizar".
    let timer = slint::Timer::default();
    let c = ctx.clone();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(60), move || c.auto_refresh());

    ui.run()
}
