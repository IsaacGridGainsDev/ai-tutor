mod backend;

use tauri::{Manager, RunEvent};

fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &app.config().app.windows[0])?;
    // Debug builds only: lets e2e scripts drive the real window over CDP.
    #[cfg(debug_assertions)]
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
            let backend = backend::Backend::start(app.handle());
            app.manage(backend);
            create_main_window(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend::backend_info])
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<backend::Backend>().shutdown();
        }
    });
}
