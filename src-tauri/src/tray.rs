//! Notification-area icon: closing the window only hides it, so quota readouts and reset alerts go on.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Window, WindowEvent};
use tauri_plugin_notification::NotificationExt;

const WINDOW: &str = "main";

pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Ouvrir", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Trigenys Claude Accounts Manager")
        .menu(&Menu::with_items(app, &[&open, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Brings the main window back (tray click, menu, or a second launch of the app).
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Closing the window hides it; the first time, say where the app went.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    static HINTED: AtomicBool = AtomicBool::new(false);
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
        if !HINTED.swap(true, Ordering::Relaxed) {
            let _ = window
                .app_handle()
                .notification()
                .builder()
                .title("Toujours actif en arrière-plan")
                .body("Les quotas restent suivis. Rouvrez l'app depuis son icône près de l'horloge ; « Quitter » dans son menu pour l'arrêter.")
                .show();
        }
    }
}
