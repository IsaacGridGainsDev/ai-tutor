mod ai;
mod backend;
mod library;
mod model;
mod visual;

use tauri::{Manager, RunEvent};

fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &app.config().app.windows[0])?;
    // Never in shipped builds: lets e2e scripts drive the real window over CDP.
    #[cfg(any(debug_assertions, feature = "e2e"))]
    let builder = match std::env::var("TUTOR_E2E_CDP_PORT") {
        Ok(port) => builder.additional_browser_args(&format!(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
        )),
        Err(_) => builder,
    };
    builder.build()?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            // Before the backend starts, so the two never bring the database up to date at once.
            // A failure here is reported by the first screen that reads the library.
            let _ = visual::with_library(tutor_core::db::migrate);
            let backend = backend::Backend::start(app.handle());
            app.manage(backend);
            app.manage(ai::Models::new());
            create_main_window(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::backend_info,
            model::room_model,
            visual::book_visuals,
            visual::diagram_delete,
            visual::climate_list,
            visual::climate_candidates,
            visual::climate_save,
            visual::climate_delete,
            visual::climate_parse,
            library::books_list,
            library::book_set_collections,
            library::book_structure,
            library::section_passages,
            library::collections_list,
            library::collection_create,
            library::collection_delete,
            library::search_library,
            library::search_recent,
            library::search_remember,
            library::search_clear_recent,
            library::settings_get,
            library::settings_put,
            ai::ai_status,
            ai::ask,
            ai::model_pull,
            ai::stop_request,
            ai::model_delete,
            ai::book_concepts,
            ai::concepts_enrich,
            ai::section_draw,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<backend::Backend>().shutdown();
        }
    });
}
