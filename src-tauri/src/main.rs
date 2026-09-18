#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod doc_generator;
mod ea;
mod legacy;
mod project;
mod python_runtime;
mod runtime;
mod renderer;
mod visio;
use tauri::Manager;

fn main() {
    if std::env::args().skip(1).any(|arg| arg == "--offline-runtime-check") {
        match python_runtime::offline_check() {
            Ok(report) => {
                if let Err(error) = tauri::async_runtime::block_on(renderer::offline_check()) {
                    eprintln!("Offline renderer check failed: {error}");
                    std::process::exit(1);
                }
                let mut report: serde_json::Value = serde_json::from_str(&report).expect("Runtime smoke returns JSON");
                report["localRenderer"] = serde_json::json!("ok");
                report["bundledFrontend"] = serde_json::json!(python_runtime::bundled_frontend());
                report["debugBuild"] = serde_json::json!(cfg!(debug_assertions));
                println!("{report}");
            }
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let runtime = runtime::Runtime::open(&directory.join("runtime.sqlite3"))
                .map_err(std::io::Error::other)?;
            app.manage(runtime);
            app.manage(ai::Ai::new(&directory));
            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            commands::generate_diagram,
            commands::generate_documentation,
            commands::export_to_tool,
            commands::get_template,
            project::save_project,
            project::load_project,
            project::list_project_history,
            project::load_project_revision,
            legacy::import_legacy_project,
            runtime::open_edit_session,
            runtime::apply_project_edit,
            runtime::create_generation_job,
            runtime::run_generation_job,
            runtime::list_generation_jobs,
            runtime::cancel_generation_job,
            runtime::discard_generation_job,
            runtime::accept_generation_job,
            runtime::recover_generation_job,
            renderer::render_local_diagram,
            ai::settings::ai_status,
            ai::settings::ai_configure,
            ai::settings::ai_set_key,
            ai::settings::ai_ollama_models,
            ai::settings::ai_test_provider,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Providers are also in a kill-on-close job; this stops them before the window is gone.
            if let tauri::RunEvent::Exit = event {
                app.state::<ai::Ai>().cancel_all();
            }
        });
}
