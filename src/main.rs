mod app;
mod assets;
mod render;
mod state;
mod ui;
mod utils;

use std::backtrace::Backtrace;
use std::sync::OnceLock;

use winit::event_loop::{ControlFlow, EventLoop};

pub(crate) static EVENT_PROXY: OnceLock<winit::event_loop::EventLoopProxy<UserEvent>> =
    OnceLock::new();

#[cfg(not(target_os = "android"))]
fn main() {
    #[cfg(target_os = "linux")]
    utils::linux::silence_glib_logs();

    std::panic::set_hook(Box::new(|info| {
        let bt = Backtrace::force_capture();
        let msg = format!("panic: {info}\nbacktrace:\n{bt}");
        eprintln!("{msg}");

        let path = dirs::download_dir()
            .unwrap_or_default()
            .join("erh_smartboard_backtrace.txt");
        let _ = std::fs::write(&path, &msg);

        rfd::MessageDialog::new()
            .set_title("崩溃")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{info}\n\nstacktrace → {}", path.display()))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }));

    pollster::block_on(run_desktop());
}

enum UserEvent {
    TrayIconEvent(tray_icon::TrayIconEvent),
    FileDialogResult {
        path: Option<std::path::PathBuf>,
        action: app::FileDialogAction,
        page_index: Option<usize>,
    },
}

#[cfg(not(target_os = "android"))]
async fn run_desktop() {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    EVENT_PROXY.set(proxy.clone()).ok();
    let tray_proxy = proxy.clone();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = tray_proxy.send_event(UserEvent::TrayIconEvent(event));
    }));
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = app::App::new();
    event_loop.run_app(&mut app).expect("failed to run app");
}
