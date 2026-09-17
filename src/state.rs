//! The state shared by every event-loop callback.

use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    desktop::{Space, Window},
    input::{SeatHandler, SeatState, keyboard::KeyboardHandle},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::workspaces::Workspace;

/// All mutable compositor state lives here.
///
/// Keeping this in one ordinary struct is intentional. `calloop` passes a
/// mutable reference to it into callbacks, so the first version does not need
/// `Arc<Mutex<_>>` for its own state.
pub struct Villain {
    pub tty: Option<crate::tty::Tty>,
    pub dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    pub display_handle: DisplayHandle,
    pub socket_name: OsString,
    pub start_time: Instant,
    pub loop_signal: LoopSignal,

    /// The desktop plane: windows are mapped here and later rendered here.
    pub space: Space<Window>,

    // Protocol state is kept separate because each Smithay handler owns one
    // protocol's bookkeeping and exposes it through a trait implementation.
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub data_device_state: DataDeviceState,
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,

    // XDG shell dispatch needs to know what a compositor considers a seat,
    // even before Villain creates real keyboard or pointer devices.
    pub seat_state: SeatState<Self>,
    pub keyboard: KeyboardHandle<Self>,
    pub pointer: smithay::input::pointer::PointerHandle<Self>,
    pub pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    pub workspaces: [Workspace; 10],
    pub active_workspace: usize,
    pub output_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    pub children: Vec<(usize, std::process::Child)>,
    pub suppressed_keys: std::collections::HashSet<smithay::input::keyboard::Keycode>,
    pub host_focused: bool,
    pub pressed_buttons: std::collections::HashSet<u32>,
}

impl Villain {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>) -> Self {
        let display_handle = display.handle();
        let socket_name = init_wayland_listener(display, event_loop);
        // GTK only exposes a default GdkSeat after it has both wl_seat and
        // wl_data_device_manager. Advertise the selection manager first so
        // clients can construct a complete seat as globals arrive.
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "villain");
        let keyboard = seat
            .add_keyboard(Default::default(), 200, 25)
            .expect("initialize keyboard");
        let pointer = seat.add_pointer();

        Self {
            tty: None,
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            display_handle: display_handle.clone(),
            socket_name,
            start_time: Instant::now(),
            loop_signal: event_loop.get_signal(),
            space: Space::default(),
            compositor_state: CompositorState::new::<Self>(&display_handle),
            xdg_shell_state: XdgShellState::new::<Self>(&display_handle),
            shm_state: ShmState::new::<Self>(&display_handle, vec![]),
            data_device_state,
            output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&display_handle),
            seat_state,
            keyboard,
            pointer,
            pointer_location: (0.0, 0.0).into(),
            workspaces: std::array::from_fn(|_| Workspace::default()),
            active_workspace: 0,
            output_size: (800, 600).into(),
            children: Vec::new(),
            suppressed_keys: Default::default(),
            host_focused: true,
            pressed_buttons: Default::default(),
        }
    }
}

impl SeatHandler for Villain {
    type KeyboardFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    type PointerFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    type TouchFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

/// Data attached to each connected client.
///
/// Smithay needs this because compositor state is associated with a client,
/// not only with the global compositor state.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

fn init_wayland_listener(
    display: Display<Villain>,
    event_loop: &mut EventLoop<Villain>,
) -> OsString {
    let listening_socket = ListeningSocketSource::new_auto().expect("create Wayland socket");
    let socket_name = listening_socket.socket_name().to_os_string();
    let loop_handle = event_loop.handle();

    loop_handle
        .insert_source(listening_socket, move |client_stream, _, state| {
            state
                .display_handle
                .insert_client(client_stream, Arc::new(ClientState::default()))
                .expect("insert Wayland client");
        })
        .expect("register Wayland socket");

    loop_handle
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            move |_, display, state| {
                // The Display is owned by this event source for the lifetime
                // of the event loop. calloop gives us a mutable source value.
                unsafe { display.get_mut().dispatch_clients(state) }?;
                Ok(PostAction::Continue)
            },
        )
        .expect("register Wayland display");

    socket_name
}
