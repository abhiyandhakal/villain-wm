//! The smallest set of protocol handlers needed to show client windows.

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState, with_states},
        output::OutputHandler,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
    },
};
use smithay::{reexports::wayland_server::protocol::wl_seat, utils::Serial};

use crate::state::{ClientState, Villain};

impl CompositorHandler for Villain {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        // This turns a client's wl_buffer commit into the state Smithay's
        // renderer can later inspect.
        on_commit_buffer_handler::<Self>(surface);

        // The first commit is the handshake: we tell the client which state
        // the compositor accepts, then the client can commit its first buffer.
        if let Some(window) = self
            .workspaces
            .iter()
            .flatten()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
        {
            // Update the window's bounding box from the newly committed
            // surface. Space uses this geometry to decide what gets rendered
            // on each output.
            window.on_commit();

            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .expect("XDG toplevel data")
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });

            if !initial_configure_sent {
                window.toplevel().unwrap().send_configure();
            }
            self.refresh_pointer(0);
        }
    }
}

impl BufferHandler for Villain {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Villain {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl OutputHandler for Villain {}

impl XdgShellHandler for Villain {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        tracing::info!("client created a toplevel surface");
        self.add_window(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        for slot in &mut self.workspaces {
            if slot
                .as_ref()
                .is_some_and(|window| window.toplevel() == Some(&surface))
                && let Some(window) = slot.take()
            {
                self.space.unmap_elem(&window);
            }
        }
        self.focus_active();
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}
}

// Smithay's delegation macro connects the protocol objects to the handlers
// above. It is intentionally at the bottom: the implementations are easier to
// find before the generated dispatch glue.
smithay::delegate_compositor!(Villain);
smithay::delegate_output!(Villain);
smithay::delegate_seat!(Villain);
smithay::delegate_shm!(Villain);
smithay::delegate_xdg_shell!(Villain);
