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

pub struct UnmanagedWindow {
    pub window: Window,
    pub workspace: usize,
    pub parent: Option<Window>,
}

impl Villain {
    /// XWayland routes pointer events using its own X11 stacking order.
    /// Keep visible windows above windows retained on other workspaces.
    pub fn sync_x11_stacking(&mut self) {
        let Some(xwm) = self.xwm.as_mut() else {
            return;
        };
        for surface in self.space.elements().filter_map(Window::x11_surface) {
            if let Err(error) = xwm.raise_window(surface) {
                tracing::warn!(%error, "could not synchronize X11 stacking");
            }
        }
    }

    pub fn refresh_unmanaged_x11_windows(&mut self) {
        for entry in &self.unmanaged_x11_windows {
            self.space.unmap_elem(&entry.window);
            if entry.workspace == self.active_workspace
                && entry
                    .parent
                    .as_ref()
                    .is_none_or(|parent| self.space.element_location(parent).is_some())
            {
                let geometry = entry.window.x11_surface().unwrap().geometry();
                self.space
                    .map_element(entry.window.clone(), geometry.loc, false);
            }
        }
    }
}

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
            // X11 focus can be selected before XWayland creates its wl_surface.
            // Repeat the enter now so Wayland keyboard events reach that client.
            let focus = crate::focus::KeyboardFocus::for_window(&window);
            if self.keyboard.current_focus() == focus {
                let keyboard = self.keyboard.clone();
                keyboard.set_focus(self, None, smithay::utils::SERIAL_COUNTER.next_serial());
                keyboard.set_focus(self, focus, smithay::utils::SERIAL_COUNTER.next_serial());
            }
            self.refresh_pointer_surface(0);
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

    fn map_window_notify(&mut self, _xwm: XwmId, _window: X11Surface) {
        // Mapping can raise a background window after map_window_request.
        self.sync_x11_stacking();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let parent = surface
            .is_transient_for()
            .and_then(|id| self.window_for_x11_id(id))
            .or_else(|| {
                self.keyboard.current_focus().and_then(|focus| {
                    self.space
                        .elements()
                        .find(|window| {
                            crate::focus::KeyboardFocus::for_window(window).as_ref() == Some(&focus)
                        })
                        .cloned()
                })
            });
        let workspace = parent
            .as_ref()
            .and_then(|window| self.workspace_for_window(window))
            .unwrap_or(self.active_workspace);
        let window = Window::new_x11_window(surface);
        self.unmanaged_x11_windows.push(UnmanagedWindow {
            window,
            workspace,
            parent,
        });
        self.refresh_unmanaged_x11_windows();
        self.sync_x11_stacking();
        self.refresh_pointer(0);
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
        _geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if !surface.is_override_redirect() {
            return;
        }
        if self.window_for_x11_surface(&surface).is_some() {
            self.refresh_unmanaged_x11_windows();
            if self
                .window_for_x11_surface(&surface)
                .is_some_and(|window| self.space.element_location(&window).is_none())
            {
                self.sync_x11_stacking();
            }
            self.refresh_pointer(0);
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
            .is_some_and(|focus| matches!(focus, crate::focus::KeyboardFocus::X11(_)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::{
        wayland_server::Display,
        x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _},
    };
    use std::time::{Duration, Instant};

    fn pump_until(
        event_loop: &mut EventLoop<'static, Villain>,
        state: &mut Villain,
        mut done: impl FnMut(&Villain) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(state) {
            assert!(Instant::now() < deadline, "XWayland test timed out");
            event_loop
                .dispatch(Duration::from_millis(5), state)
                .unwrap();
            state.space.refresh();
            state.display_handle.flush_clients().unwrap();
        }
    }

    /// Uses a private, headless compositor and a real XWayland server. Run with
    /// an isolated XDG_RUNTIME_DIR; no desktop session or GPU is required.
    #[test]
    #[ignore = "requires Xwayland and an isolated XDG_RUNTIME_DIR"]
    fn x11_workspace_input_isolation() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut state = Villain::new(
            &mut event_loop,
            Display::new().unwrap(),
            crate::config::RuntimeConfig::load().unwrap(),
        );
        let output = smithay::output::Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
            },
        );
        output.create_global::<Villain>(&state.display_handle);
        let mode = smithay::output::Mode {
            size: (800, 600).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        state.space.map_output(&output, (0, 0));
        init(&mut event_loop, &mut state);
        pump_until(&mut event_loop, &mut state, |state| {
            state.xwayland_display.is_some()
        });
        let display = format!(":{}", state.xwayland_display.unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            sender
                .send(smithay::reexports::x11rb::connect(Some(&display)))
                .unwrap()
        });
        let mut connection = None;
        pump_until(&mut event_loop, &mut state, |_| {
            connection = receiver.try_recv().ok();
            connection.is_some()
        });
        let (conn, screen) = connection.unwrap().unwrap();
        let root = conn.setup().roots[screen].root;
        let create = |override_redirect: bool, parent: Option<u32>| {
            let id = conn.generate_id().unwrap();
            conn.create_window(
                0,
                id,
                root,
                0,
                0,
                800,
                600,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new()
                    .override_redirect(u32::from(override_redirect))
                    .background_pixel(0x123456)
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::KEY_PRESS),
            )
            .unwrap();
            if let Some(parent) = parent {
                conn.change_property32(
                    PropMode::REPLACE,
                    id,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    &[parent],
                )
                .unwrap();
            }
            conn.map_window(id).unwrap();
            conn.flush().unwrap();
            id
        };
        let first = create(false, None);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(first)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        state.switch_workspace(4);
        let second = create(false, None);
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(second)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        let popup = create(true, Some(second));
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(popup)
                .is_some_and(|window| window.x11_surface().unwrap().wl_surface().is_some())
        });
        state.pointer_location = (100.0, 100.0).into();

        for (workspace, expected, hidden) in
            [(0, first, second), (4, second, first), (0, first, second)]
        {
            state.switch_workspace(workspace);
            state.display_handle.flush_clients().unwrap();
            assert_eq!(
                conn.get_input_focus().unwrap().reply().unwrap().focus,
                expected
            );
            let pointer_target = if workspace == 4 { popup } else { expected };
            let window = state.window_for_x11_id(pointer_target).unwrap();
            assert_eq!(
                state.pointer.current_focus(),
                window.x11_surface().unwrap().wl_surface()
            );
            assert!(
                state
                    .space
                    .element_location(&state.window_for_x11_id(hidden).unwrap())
                    .is_none()
            );
            let popup_window = state.window_for_x11_id(popup).unwrap();
            assert_eq!(
                state.space.element_location(&popup_window).is_some(),
                workspace == 4
            );
            let tree = conn.query_tree(root).unwrap().reply().unwrap().children;
            let frame = |id| {
                state
                    .window_for_x11_id(id)
                    .unwrap()
                    .x11_surface()
                    .unwrap()
                    .mapped_window_id()
                    .unwrap()
            };
            assert!(
                tree.iter().position(|id| *id == frame(expected))
                    > tree.iter().position(|id| *id == frame(hidden))
            );
            while conn.poll_for_event().unwrap().is_some() {}
            let pointer = state.pointer.clone();
            for button_state in [
                smithay::backend::input::ButtonState::Pressed,
                smithay::backend::input::ButtonState::Released,
            ] {
                pointer.button(
                    &mut state,
                    &smithay::input::pointer::ButtonEvent {
                        serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                        time: 1,
                        button: 0x110,
                        state: button_state,
                    },
                );
            }
            pointer.frame(&mut state);
            state.display_handle.flush_clients().unwrap();
            let mut clicked = None;
            pump_until(&mut event_loop, &mut state, |_| {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    if let smithay::reexports::x11rb::protocol::Event::ButtonPress(event) = event {
                        clicked = Some(event.event);
                    }
                }
                clicked.is_some()
            });
            assert_eq!(
                clicked,
                Some(pointer_target),
                "click reached the wrong X11 window"
            );
            let keyboard = state.keyboard.clone();
            for key_state in [
                smithay::backend::input::KeyState::Pressed,
                smithay::backend::input::KeyState::Released,
            ] {
                keyboard.input::<(), _>(
                    &mut state,
                    38u32.into(),
                    key_state,
                    smithay::utils::SERIAL_COUNTER.next_serial(),
                    2,
                    |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                );
            }
            state.display_handle.flush_clients().unwrap();
            let mut typed = None;
            pump_until(&mut event_loop, &mut state, |_| {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    if let smithay::reexports::x11rb::protocol::Event::KeyPress(event) = event {
                        typed = Some(event.event);
                    }
                }
                typed.is_some()
            });
            assert_eq!(typed, Some(expected), "key reached the wrong X11 window");
        }
        // A configure event from a hidden popup must not remap it or steal focus.
        conn.configure_window(popup, &ConfigureWindowAux::new().x(20))
            .unwrap();
        conn.flush().unwrap();
        pump_until(&mut event_loop, &mut state, |state| {
            state
                .window_for_x11_id(popup)
                .unwrap()
                .x11_surface()
                .unwrap()
                .geometry()
                .loc
                .x
                == 20
        });
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(popup).unwrap())
                .is_none()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
        // A background app can create a popup after we switch away.
        let late_popup = create(true, Some(second));
        pump_until(&mut event_loop, &mut state, |state| {
            state.window_for_x11_id(late_popup).is_some()
        });
        assert!(
            state
                .space
                .element_location(&state.window_for_x11_id(late_popup).unwrap())
                .is_none()
        );
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
        state.pressed_buttons.insert(0x110);
        state.pointer.clone().button(
            &mut state,
            &smithay::input::pointer::ButtonEvent {
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: 3,
                button: 0x110,
                state: smithay::backend::input::ButtonState::Pressed,
            },
        );
        assert!(state.pointer.is_grabbed());
        assert!(state.minimize_focused_window());
        assert!(!state.pointer.is_grabbed());
        assert!(state.pressed_buttons.is_empty());
        assert!(state.keyboard.current_focus().is_none());
        assert!(state.pointer.current_focus().is_none());
        assert!(state.restore_last_minimized_window());
        assert_eq!(
            conn.get_input_focus().unwrap().reply().unwrap().focus,
            first
        );
    }
}
