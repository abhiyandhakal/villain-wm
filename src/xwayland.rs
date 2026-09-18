//! XWayland server and X11 window-manager integration.

use std::{os::fd::OwnedFd, process::Stdio};

use smithay::{
    desktop::Window,
    reexports::calloop::EventLoop,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, request_data_device_client_selection,
                set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, X11Window, XwmId},
    },
};

use crate::{state::Villain, workspaces::WindowAction};

/// Start XWayland without making it a prerequisite for native Wayland clients.
pub fn init(event_loop: &mut EventLoop<'static, Villain>, state: &mut Villain) {
    let environment = state.config.environment.clone();
    let Ok((xwayland, client)) = XWayland::spawn(
        &state.display_handle,
        None,
        environment,
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) else {
        tracing::warn!("XWayland is unavailable; continuing as a Wayland-only compositor");
        return;
    };

    let loop_handle = event_loop.handle();
    let xwm_handle = loop_handle.clone();
    if let Err(error) = loop_handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready {
            x11_socket,
            display_number,
        } => match X11Wm::start_wm(xwm_handle.clone(), x11_socket, client.clone()) {
            Ok(xwm) => {
                state.xwm = Some(xwm);
                state.xwayland_display = Some(display_number);
                state
                    .config
                    .environment
                    .insert("DISPLAY".into(), format!(":{display_number}"));
                if state.owns_session {
                    crate::session::activate(state, false);
                }
                tracing::info!(display = %format_args!(":{display_number}"), "XWayland is ready");
            }
            Err(error) => tracing::error!(%error, "could not start the XWayland window manager"),
        },
        XWaylandEvent::Error => {
            tracing::error!("XWayland exited before it became ready");
        }
    }) {
        tracing::error!(%error, "could not register XWayland with the event loop");
    }
}

impl XWaylandShellHandler for Villain {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        _surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        window: X11Surface,
    ) {
        if let Some(window) = self.window_for_x11_surface(&window) {
            window.on_commit();
            self.request_repaint();
        }
    }
}

impl XwmHandler for Villain {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        let state = self.xwm.as_mut().expect("XWM callback without an XWM");
        assert_eq!(state.id(), xwm);
        state
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(error) = window.set_mapped(true) {
            tracing::warn!(%error, window = window.window_id(), "could not map X11 window");
            return;
        }
        self.add_x11_window(window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let geometry = surface.geometry();
        let window = Window::new_x11_window(surface);
        self.space.map_element(window.clone(), geometry.loc, true);
        self.unmanaged_x11_windows.push(window);
        self.request_repaint();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<u32>,
        height: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if !window.is_override_redirect() && self.window_for_x11_surface(&window).is_some() {
            self.relayout_active_workspace();
            return;
        }

        let mut geometry = window.geometry();
        geometry.loc.x = x.unwrap_or(geometry.loc.x);
        geometry.loc.y = y.unwrap_or(geometry.loc.y);
        geometry.size.w = width
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(geometry.size.w);
        geometry.size.h = height
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(geometry.size.h);
        if let Err(error) = window.configure(geometry) {
            tracing::warn!(%error, window = window.window_id(), "could not configure X11 window");
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if !surface.is_override_redirect() {
            return;
        }
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.space.map_element(window, geometry.loc, true);
            self.request_repaint();
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11_surface(&surface) {
            self.apply_window_action(&window, WindowAction::Minimize);
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.keyboard
            .current_focus()
            .and_then(|surface| self.window_for_surface(&surface))
            .is_some_and(|window| window.is_x11())
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let result = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|error| error.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|error| error.to_string()),
        };
        if let Err(error) = result {
            tracing::warn!(%error, ?selection, "could not transfer Wayland selection to XWayland");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                clear_data_device_selection(&self.display_handle, &self.seat)
            }
            SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
        }
    }
}

smithay::delegate_xwayland_shell!(Villain);
