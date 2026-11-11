//! The shooter's client: one crate for the native window (wgpu on Vulkan/Metal/DX12) and the
//! browser (WebGPU, `wasm32`), rendered by render-engine, with input-bindings controls and
//! game-audio footsteps over the shared `shooter-sim` simulation.

pub mod app;
pub mod audio;
pub mod controls;
pub mod gpu;
pub mod scene;

use winit::event_loop::EventLoop;

/// Opens the window (or the page's canvas) and runs the game until it closes.
///
/// # Panics
///
/// If the platform has no event loop.
pub fn run() {
    let event_loop = EventLoop::<app::GpuReady>::with_user_event()
        .build()
        .expect("an event loop");
    let app = app::App::new(event_loop.create_proxy());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        if let Err(error) = event_loop.run_app(&mut app) {
            fail(&error.to_string());
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

pub(crate) fn log(message: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{message}");
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&message.into());
}

/// A fatal error: logged, and shown on the page in the browser.
pub(crate) fn fail(message: &str) {
    log(message);
    #[cfg(target_arch = "wasm32")]
    web::show_error(message);
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod web {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(start)]
    pub fn start() {
        console_error_panic_hook::set_once();
        super::run();
    }

    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    /// Whether the page holds Pointer Lock (the browser releases it on Escape).
    pub fn pointer_locked() -> bool {
        document().is_some_and(|document| document.pointer_lock_element().is_some())
    }

    pub fn show_error(message: &str) {
        if let Some(element) = document().and_then(|document| document.get_element_by_id("error")) {
            element.set_text_content(Some(message));
            let _ = element.set_attribute("data-shown", "");
        }
    }
}
