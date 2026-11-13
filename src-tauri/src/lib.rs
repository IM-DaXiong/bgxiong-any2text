mod application;
mod export;
mod provider;
mod providers;
mod workspace;

use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                for result in [window.show(), window.unminimize(), window.set_focus()] {
                    if let Err(error) = result {
                        eprintln!("Any2Text could not restore its existing window: {error}");
                    }
                }
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let workspace_root = std::env::var_os("BGXIONG_ANY2TEXT_DATA_ROOT")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?);
            let bridge_root = std::env::var_os("BGXIONG_BRIDGE_REGISTRY_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().local_data_dir()?.join("BigBear/local-bridge"));
            if !workspace_root.is_absolute() || !bridge_root.is_absolute() {
                return Err("Any2Text data and registry paths must be absolute".into());
            }
            let workspace = workspace::Workspace::open(workspace_root)?;
            let registry = bridge_root.join("registry.json");
            let provider = providers::bigbear::BigBear::new(workspace.client_key()?, registry)?;
            app.manage(application::Application::new(workspace, Arc::new(provider)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            application::connect,
            application::disconnect,
            application::provider_health,
            application::provider_appearance,
            application::launch_bigbear,
            application::projects,
            application::import_media,
            application::playback_path,
            application::relocate_media,
            application::copy_media,
            application::jobs,
            application::submit_transcription,
            application::refresh_job,
            application::retry_pending_request,
            application::control_job,
            application::retrieve,
            application::document,
            application::revisions,
            application::checkout_revision,
            application::save_correction,
            application::preview_export,
            application::export_document,
            application::submit_organize,
            application::candidates,
            application::save_candidate,
            application::accept_candidate,
            application::preview_candidate,
            application::export_candidate,
        ])
        .run(tauri::generate_context!())
        .expect("Any2Text desktop startup failed");
}
