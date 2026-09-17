//! Ordered workspaces with a master-and-stack layout.
use crate::state::Villain;
use smithay::{
    desktop::{Window, WindowSurfaceType},
    input::pointer::MotionEvent,
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::Resource},
    utils::{Point, SERIAL_COUNTER, Size},
    wayland::shell::xdg::ToplevelSurface,
};
use std::process::Command;

#[derive(Default)]
pub struct Workspace {
    windows: Vec<WorkspaceWindow>,
    minimized_history: Vec<Window>,
}

struct WorkspaceWindow {
    window: Window,
    minimized: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum WindowAction {
    Close,
    Minimize,
}

fn master_stack_layout(
    output: Size<i32, smithay::utils::Logical>,
    window_count: usize,
) -> Vec<(
    Point<i32, smithay::utils::Logical>,
    Size<i32, smithay::utils::Logical>,
)> {
    match window_count {
        0 => Vec::new(),
        1 => vec![((0, 0).into(), output)],
        _ => {
            let master_width = output.w / 2;
            let stack_width = output.w - master_width;
            let stack_count = window_count as i32 - 1;
            let stack_height = output.h / stack_count;
            let mut result = Vec::with_capacity(window_count);
            result.push(((0, 0).into(), (master_width, output.h).into()));
            for index in 0..stack_count {
                let y = index * stack_height;
                let height = if index + 1 == stack_count {
                    output.h - y
                } else {
                    stack_height
                };
                result.push(((master_width, y).into(), (stack_width, height).into()));
            }
            result
        }
    }
}

impl Villain {
    pub fn launch_terminal(&mut self) {
        self.reap_children();
        let workspace = self.active_workspace;
        let program = std::env::var_os("VILLAIN_TERMINAL").unwrap_or_else(|| "kitty".into());
        match Command::new(&program)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("WAYLAND_SOCKET")
            // A shell started from a desktop terminal can inherit the host's
            // X11 display and a forced GTK backend. Neither describes the
            // session that Villain is providing to this child.
            .env_remove("DISPLAY")
            .env_remove("GDK_BACKEND")
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
    fn configure_window(window: &Window, size: Size<i32, smithay::utils::Logical>) {
        let surface = window.toplevel().unwrap();
        surface.with_pending_state(|pending| {
            pending.size = Some(size);
            pending.states.unset(xdg_toplevel::State::Fullscreen);
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
        let window = Window::new_wayland_window(surface);
        self.workspaces[index].windows.push(WorkspaceWindow {
            window,
            minimized: false,
        });
        if index == self.active_workspace {
            self.relayout_active_workspace();
        }
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
        self.relayout_active_workspace();
    }

    pub fn relayout_active_workspace(&mut self) {
        let old: Vec<_> = self.space.elements().cloned().collect();
        for window in old {
            self.space.unmap_elem(&window);
        }

        let visible: Vec<_> = self.workspaces[self.active_workspace]
            .windows
            .iter()
            .filter(|entry| !entry.minimized)
            .map(|entry| entry.window.clone())
            .collect();
        let visible_count = visible.len();
        for (window, (location, size)) in visible
            .into_iter()
            .zip(master_stack_layout(self.output_size, visible_count))
        {
            Self::configure_window(&window, size);
            self.space.map_element(window, location, false);
        }
        self.refresh_pointer(0);
    }

    pub fn close_focused_window(&mut self) {
        if let Some(surface) = self.focused_toplevel() {
            self.apply_window_action(&surface, WindowAction::Close);
        }
    }

    pub fn minimize_focused_window(&mut self) {
        if let Some(surface) = self.focused_toplevel() {
            self.apply_window_action(&surface, WindowAction::Minimize);
        }
    }

    pub fn apply_window_action(&mut self, surface: &ToplevelSurface, action: WindowAction) {
        match action {
            WindowAction::Close => {
                if self.window_for_toplevel(surface).is_some() {
                    surface.send_close();
                }
            }
            WindowAction::Minimize => {
                let mut changed_active_workspace = false;
                for (index, workspace) in self.workspaces.iter_mut().enumerate() {
                    let Some(entry) = workspace
                        .windows
                        .iter_mut()
                        .find(|entry| entry.window.toplevel() == Some(surface) && !entry.minimized)
                    else {
                        continue;
                    };
                    entry.minimized = true;
                    let window = entry.window.clone();
                    workspace
                        .minimized_history
                        .retain(|candidate| candidate != &window);
                    workspace.minimized_history.push(window);
                    changed_active_workspace = index == self.active_workspace;
                    break;
                }
                if changed_active_workspace {
                    self.relayout_active_workspace();
                }
            }
        }
    }

    pub fn restore_last_minimized_window(&mut self) {
        let workspace = &mut self.workspaces[self.active_workspace];
        while let Some(window) = workspace.minimized_history.pop() {
            if let Some(entry) = workspace
                .windows
                .iter_mut()
                .find(|entry| entry.window == window && entry.minimized)
            {
                entry.minimized = false;
                self.relayout_active_workspace();
                return;
            }
        }
    }

    fn focused_toplevel(&self) -> Option<ToplevelSurface> {
        let focused = self.keyboard.current_focus()?;
        self.workspaces[self.active_workspace]
            .windows
            .iter()
            .find(|entry| entry.window.toplevel().unwrap().wl_surface() == &focused)
            .and_then(|entry| entry.window.toplevel().cloned())
    }

    fn window_for_toplevel(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .find(|entry| entry.window.toplevel() == Some(surface))
            .map(|entry| entry.window.clone())
    }

    pub fn window_for_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.windows)
            .find(|entry| entry.window.toplevel().unwrap().wl_surface() == surface)
            .map(|entry| entry.window.clone())
    }

    pub fn focus_window_at_pointer(&mut self) {
        let hit = self
            .space
            .element_under(self.pointer_location)
            .map(|(window, location)| (window.clone(), location));
        let keyboard_surface = hit
            .as_ref()
            .filter(|_| self.host_focused)
            .map(|(window, _)| window.toplevel().unwrap().wl_surface().clone());
        if self.keyboard.current_focus() != keyboard_surface {
            for entry in &self.workspaces[self.active_workspace].windows {
                let activated = keyboard_surface.as_ref().is_some_and(|surface| {
                    entry.window.toplevel().unwrap().wl_surface() == surface
                });
                entry.window.set_activated(activated);
                entry.window.toplevel().unwrap().send_pending_configure();
            }
            let keyboard = self.keyboard.clone();
            keyboard.set_focus(self, keyboard_surface, SERIAL_COUNTER.next_serial());
        }
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
        let hit = self
            .space
            .element_under(self.pointer_location)
            .map(|(window, location)| (window.clone(), location));
        let focus = if self.host_focused {
            hit.and_then(|(window, origin)| {
                window
                    .surface_under(
                        self.pointer_location - origin.to_f64(),
                        WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE,
                    )
                    .map(|(surface, location)| (surface, location.to_f64() + origin.to_f64()))
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
        self.focus_window_at_pointer();
    }

    pub fn remove_window(&mut self, surface: &ToplevelSurface) {
        for workspace in &mut self.workspaces {
            let removed: Vec<_> = workspace
                .windows
                .extract_if(.., |entry| entry.window.toplevel() == Some(surface))
                .map(|entry| entry.window)
                .collect();
            for window in removed {
                self.space.unmap_elem(&window);
                workspace
                    .minimized_history
                    .retain(|candidate| candidate != &window);
            }
        }
        self.relayout_active_workspace();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_stack_layouts_zero_to_three_windows() {
        let output = (800, 600).into();
        assert!(master_stack_layout(output, 0).is_empty());
        assert_eq!(
            master_stack_layout(output, 1),
            vec![((0, 0).into(), (800, 600).into())]
        );
        assert_eq!(
            master_stack_layout(output, 2),
            vec![
                ((0, 0).into(), (400, 600).into()),
                ((400, 0).into(), (400, 600).into()),
            ]
        );
        assert_eq!(
            master_stack_layout(output, 3),
            vec![
                ((0, 0).into(), (400, 600).into()),
                ((400, 0).into(), (400, 300).into()),
                ((400, 300).into(), (400, 300).into()),
            ]
        );
    }

    #[test]
    fn stack_absorbs_integer_remainders() {
        let layout = master_stack_layout((801, 601).into(), 4);
        assert_eq!(layout[0], ((0, 0).into(), (400, 601).into()));
        assert_eq!(layout[3], ((400, 400).into(), (401, 201).into()));
    }
}
