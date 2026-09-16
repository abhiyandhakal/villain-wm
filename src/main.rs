//! Villain, a deliberately small Smithay compositor.
//!
//! The first version is a nested compositor: Smithay opens a window on the
//! existing desktop, and Villain serves Wayland clients inside that window.
//! This keeps the first experiment safe to run while exposing the same core
//! pieces that a direct-to-DRM compositor will eventually need.

mod handlers;
mod keybinds;
mod render;
mod state;
mod workspaces;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use state::Villain;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    // calloop owns the central event loop. Every event source—Wayland client
    // requests, the nested window, and later input devices—feeds into it.
    let mut event_loop: EventLoop<Villain> = EventLoop::try_new()?;
    let display = Display::new()?;
    let mut state = Villain::new(&mut event_loop, display);

    render::init_winit(&mut event_loop, &mut state)?;

    tracing::info!(socket = ?state.socket_name, "Villain is ready");
    event_loop.run(
        Some(std::time::Duration::from_millis(16)),
        &mut state,
        |state| {
            state.reap_children();
            state.space.refresh();
            let _ = state.display_handle.flush_clients();
        },
    )?;

    Ok(())
}

fn init_logging() {
    let subscriber = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "villain=info".into()),
    );
    subscriber.init();
}
