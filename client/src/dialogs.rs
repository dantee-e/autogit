//! Native file dialogs, opened the way each OS requires.
//!
//! - macOS: AppKit only allows dialogs on the main thread, so we run rfd's
//!   async dialog as a future on the Slint event loop (`slint::spawn_local`).
//! - Windows / Linux: a blocking dialog on a helper thread, so the window keeps
//!   repainting while the dialog is open.
//!
//! In every case `on_result` runs on the UI thread, and only if the user picked
//! something.

use std::path::PathBuf;

pub enum Dialog {
    PickFile { title: &'static str },
    PickFolder { title: &'static str },
    SaveFile { file_name: String },
}

pub fn ask(dialog: Dialog, on_result: impl FnOnce(PathBuf) + Send + 'static) {
    #[cfg(target_os = "macos")]
    {
        let fut = async move {
            let picked = match dialog {
                Dialog::PickFile { title } => rfd::AsyncFileDialog::new().set_title(title).pick_file().await,
                Dialog::PickFolder { title } => rfd::AsyncFileDialog::new().set_title(title).pick_folder().await,
                Dialog::SaveFile { file_name } => rfd::AsyncFileDialog::new().set_file_name(&file_name).save_file().await,
            };
            if let Some(handle) = picked {
                on_result(handle.path().to_path_buf());
            }
        };
        if let Err(e) = slint::spawn_local(fut) {
            eprintln!("could not open file dialog: {e}");
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        std::thread::spawn(move || {
            let picked = match dialog {
                Dialog::PickFile { title } => rfd::FileDialog::new().set_title(title).pick_file(),
                Dialog::PickFolder { title } => rfd::FileDialog::new().set_title(title).pick_folder(),
                Dialog::SaveFile { file_name } => rfd::FileDialog::new().set_file_name(&file_name).save_file(),
            };
            if let Some(path) = picked {
                let _ = slint::invoke_from_event_loop(move || on_result(path));
            }
        });
    }
}
