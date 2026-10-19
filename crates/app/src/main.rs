//! e6paint entry points: a native window, and the web canvas built by trunk.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "e6paint",
        options,
        Box::new(|cc| {
            let app = e6_app::E6App::new(cc)?;
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    console_error_panic_hook::set_once();
    eframe::WebLogger::init(log::LevelFilter::Info).ok();

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("e6paint_canvas"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            log::error!("canvas #e6paint_canvas not found in the document");
            return;
        };
        if let Err(err) = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| {
                    let app = e6_app::E6App::new(cc)?;
                    Ok(Box::new(app))
                }),
            )
            .await
        {
            log::error!("failed to start eframe: {err:?}");
        }
    });
}
