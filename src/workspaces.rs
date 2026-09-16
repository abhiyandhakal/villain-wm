//! Ten single-window workspaces. Only the active window is mapped in Space.
use crate::state::Villain;
use smithay::{
    desktop::{Window, WindowSurfaceType},
    input::pointer::MotionEvent,
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::Resource},
    utils::SERIAL_COUNTER,
    wayland::shell::xdg::ToplevelSurface,
};
use std::process::Command;

impl Villain {
    pub fn launch_terminal(&mut self) {
        self.reap_children();
        let workspace = self.active_workspace;
        if self.workspaces[workspace].is_some()
            || self.children.iter().any(|(index, _)| *index == workspace)
        {
            return;
        }
        let program = std::env::var_os("VILLAIN_TERMINAL").unwrap_or_else(|| "kitty".into());
        match Command::new(&program)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("WAYLAND_SOCKET")
            .spawn()
        {
            Ok(child) => {
                tracing::info!(
                    pid = child.id(),
                    workspace = workspace + 1,
                    "terminal launched"
                );
                self.children.push((workspace, child));
            }
            Err(error) => tracing::error!(?program, %error, "terminal launch failed"),
        }
    }
    pub fn reap_children(&mut self) {
        self.children
            .retain_mut(|(_, child)| match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(pid = child.id(), %status, "launched app exited");
                    false
                }
                Ok(None) => true,
                Err(error) => {
                    tracing::warn!(%error, "could not reap app");
                    true
                }
            });
    }
    pub fn configure_window(&self, window: &Window) {
        let surface = window.toplevel().unwrap();
        surface.with_pending_state(|pending| {
            pending.size = Some(self.output_size);
            pending.states.set(xdg_toplevel::State::Fullscreen);
        });
        surface.send_pending_configure();
    }
    pub fn add_window(&mut self, surface: ToplevelSurface) {
        // Associate a direct child with its launch workspace even after switching.
        let pid = surface
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .map(|credentials| credentials.pid as u32);
        let index = self
            .children
            .iter()
            .find(|(_, child)| Some(child.id()) == pid)
            .map(|(index, _)| *index)
            .unwrap_or(self.active_workspace);
        if self.workspaces[index].is_some() {
            surface.send_close();
            tracing::warn!(
                workspace = index + 1,
                "workspace occupied; closing extra toplevel"
            );
            return;
        }
        let window = Window::new_wayland_window(surface);
        self.configure_window(&window);
        self.workspaces[index] = Some(window);
        self.switch_workspace(self.active_workspace);
    }
    pub fn switch_workspace(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        self.release_pointer_buttons();
        let old: Vec<_> = self.space.elements().cloned().collect();
        for window in old {
            self.space.unmap_elem(&window);
            window.set_activated(false);
            window.toplevel().unwrap().send_pending_configure();
        }
        self.active_workspace = index;
        if let Some(window) = self.workspaces[index].clone() {
            self.space.map_element(window, (0, 0), true);
        }
        self.focus_active();
    }
    pub fn focus_active(&mut self) {
        let surface = self.workspaces[self.active_workspace]
            .as_ref()
            .filter(|_| self.host_focused)
            .map(|window| window.toplevel().unwrap().wl_surface().clone());
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        if let Some(window) = &self.workspaces[self.active_workspace] {
            window.toplevel().unwrap().send_pending_configure();
        }
        self.refresh_pointer(0);
    }
    /// End any drag before focus moves away from its application.
    pub fn release_pointer_buttons(&mut self) {
        let pointer = self.pointer.clone();
        for button in std::mem::take(&mut self.pressed_buttons) {
            pointer.button(
                self,
                &smithay::input::pointer::ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 0,
                    button,
                    state: smithay::backend::input::ButtonState::Released,
                },
            );
        }
        pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), 0);
        pointer.frame(self);
    }
    pub fn refresh_pointer(&mut self, time: u32) {
        let focus = if self.host_focused {
            self.workspaces[self.active_workspace]
                .as_ref()
                .and_then(|window| {
                    let origin = window.geometry().loc.to_f64();
                    window
                        .surface_under(
                            self.pointer_location + origin,
                            WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE,
                        )
                        .map(|(surface, location)| (surface, location.to_f64() - origin))
                })
        } else {
            None
        };
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }
}
