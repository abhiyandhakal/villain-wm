//! Direct Linux output: libseat owns access, GBM allocates buffers, DRM scans
//! them out. One connected connector/CRTC is used for this learning backend.
use crate::state::Villain;
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputEvent, KeyState,
            PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            Bind,
            damage::OutputDamageTracker,
            element::{
                Kind,
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    desktop::space::render_output,
    input::pointer::{AxisFrame, ButtonEvent},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device, ModeTypeFlags, connector},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, SERIAL_COUNTER, Transform},
};
use std::{error::Error, path::PathBuf, time::Duration};

type BufferedSurface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;

pub struct Tty {
    // Display resources drop before the event loop's libseat notifier.
    surface: BufferedSurface,
    pub renderer: GlesRenderer,
    drm: DrmDevice,
    output: Output,
    damage: OutputDamageTracker,
    cursor: SolidColorBuffer,
    active: bool,
    pending: bool,
    pub session: LibSeatSession,
}

pub fn init(
    event_loop: &mut EventLoop<Villain>,
    state: &mut Villain,
) -> Result<(), Box<dyn Error>> {
    let (mut session, notifier) = LibSeatSession::new().map_err(|error| format!("Cannot acquire Linux seat: {error}. Run from a logged-in local TTY with logind or seatd available."))?;
    let seat = session.seat();
    if !session.is_active() {
        return Err("Linux seat is not active; run from the active local TTY".into());
    }
    let path = match std::env::var_os("VILLAIN_DRM_DEVICE") {
        Some(path) => PathBuf::from(path),
        None => primary_gpu(&seat)?
            .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
            .ok_or("No DRM GPU found on this seat")?,
    };
    let fd = session.open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let device_id = fd.dev_id()?;
    let (mut drm, drm_notifier) = DrmDevice::new(fd.clone(), false)?;
    let resources = drm.resource_handles()?;
    let mut selected = None;
    'connectors: for handle in resources.connectors() {
        let info = drm.get_connector(*handle, true)?;
        if info.state() != connector::State::Connected {
            continue;
        }
        let Some(mode) = info
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
        else {
            continue;
        };
        for encoder in info.encoders() {
            let encoder = drm.get_encoder(*encoder)?;
            for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                if let Ok(surface) = drm.create_surface(crtc, mode, &[*handle]) {
                    selected = Some((surface, mode, info));
                    break 'connectors;
                }
            }
        }
    }
    let (surface, mode, connector) = selected
        .ok_or("No connected display with a usable CRTC; connect a monitor before starting")?;
    let gbm = GbmDevice::new(fd)?;
    // EGL retains the cloned GBM native display. All rendering stays on the
    // event-loop thread, which owns the context for its complete lifetime.
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    let renderer = unsafe { GlesRenderer::new(context)? };
    // Advertise GPU-buffer import to EGL clients such as Kitty.
    use smithay::{backend::renderer::ImportDma, wayland::dmabuf::DmabufFeedbackBuilder};
    let feedback = DmabufFeedbackBuilder::new(device_id, renderer.dmabuf_formats()).build()?;
    state
        .dmabuf_state
        .create_global_with_default_feedback::<Villain>(&state.display_handle, &feedback);
    let formats = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let surface = GbmBufferedSurface::new(
        surface,
        allocator,
        &[Fourcc::Argb8888, Fourcc::Abgr8888],
        formats,
    )?;
    let size = (i32::from(mode.size().0), i32::from(mode.size().1));
    let output = Output::new(
        format!("{:?}-{}", connector.interface(), connector.interface_id()),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Villain".into(),
            model: "DRM/KMS".into(),
        },
    );
    output.create_global::<Villain>(&state.display_handle);
    let wl_mode = Mode {
        size: size.into(),
        refresh: mode.vrefresh() as i32 * 1000,
    };
    output.change_current_state(
        Some(wl_mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(wl_mode);
    state.output_size = size.into();
    state.pointer_location = (f64::from(size.0) / 2.0, f64::from(size.1) / 2.0).into();
    state.space.map_output(&output, (0, 0));
    let damage = OutputDamageTracker::from_output(&output);
    state.tty = Some(Tty {
        surface,
        renderer,
        drm,
        output,
        damage,
        cursor: SolidColorBuffer::new((10, 16), [1.0, 1.0, 1.0, 1.0]),
        active: true,
        pending: false,
        session: session.clone(),
    });

    let mut input =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.into());
    input
        .udev_assign_seat(&seat)
        .map_err(|_| "libinput could not assign the seat")?;
    event_loop.handle().insert_source(
        LibinputInputBackend::new(input.clone()),
        |event, _, state| process_input(event, state),
    )?;
    event_loop
        .handle()
        .insert_source(drm_notifier, |event, _, state| {
            if let Some(tty) = state.tty.as_mut() {
                match event {
                    DrmEvent::VBlank(_) => {
                        if let Err(error) = tty.surface.frame_submitted() {
                            tracing::error!(%error, "page flip completion failed");
                        }
                        tty.pending = false;
                    }
                    DrmEvent::Error(error) => {
                        tracing::error!(%error, "DRM event failed");
                        state.loop_signal.stop();
                    }
                }
            }
        })?;
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                input.suspend();
                state.release_pointer_buttons();
                let keyboard = state.keyboard.clone();
                for code in keyboard.pressed_keys() {
                    keyboard.input::<(), _>(
                        state,
                        code,
                        KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        0,
                        |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                    );
                }
                state.suppressed_keys.clear();
                state.host_focused = false;
                state.focus_active();
                if let Some(tty) = state.tty.as_mut() {
                    tty.active = false;
                    // A VT switch can prevent delivery of the last flip event.
                    // Retire that in-flight buffer before starting a fresh frame.
                    let _ = tty.surface.frame_submitted();
                    tty.drm.pause();
                }
            }
            SessionEvent::ActivateSession => {
                let result = state.tty.as_mut().unwrap().drm.activate(true);
                if let Err(error) = result {
                    tracing::error!(%error, "DRM resume failed");
                    state.loop_signal.stop();
                    return;
                }
                if let Err(error) = input.resume() {
                    tracing::error!(?error, "input resume failed");
                    state.loop_signal.stop();
                    return;
                }
                let tty = state.tty.as_mut().unwrap();
                tty.surface.reset_buffers();
                tty.pending = false;
                tty.active = true;
                tty.damage = OutputDamageTracker::from_output(&tty.output);
                state.host_focused = true;
                state.focus_active();
            }
        })?;
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, state| {
            if let Some(mut tty) = state.tty.take() {
                if tty.active
                    && !tty.pending
                    && let Err(error) = tty.render(state)
                {
                    tracing::error!(%error, "DRM render failed; exiting to release the seat");
                    state.loop_signal.stop();
                }
                state.tty = Some(tty);
            }
            TimeoutAction::ToDuration(Duration::from_millis(8))
        })?;
    tracing::info!(gpu = %path.display(), ?size, "TTY backend ready; Ctrl+Alt+Backspace exits, Ctrl+Alt+F1–F12 switches VT");
    Ok(())
}

impl Tty {
    fn render(&mut self, state: &mut Villain) -> Result<(), Box<dyn Error>> {
        let (mut buffer, _age) = self.surface.next_buffer()?;
        let cursor = SolidColorRenderElement::from_buffer(
            &self.cursor,
            (
                state.pointer_location.x as i32,
                state.pointer_location.y as i32,
            ),
            1.0,
            1.0,
            Kind::Cursor,
        );
        let mut framebuffer = self.renderer.bind(&mut buffer)?;
        // Repaint the whole buffer for now: no buffer-age optimization yet.
        let result = render_output(
            &self.output,
            &mut self.renderer,
            &mut framebuffer,
            1.0,
            0,
            [&state.space],
            &[cursor],
            &mut self.damage,
            [0.08, 0.05, 0.12, 1.0],
        )?;
        let sync = result.sync.clone();
        drop(framebuffer);
        self.surface.queue_buffer(Some(sync), None, ())?;
        self.pending = true;
        for window in state.space.elements() {
            window.send_frame(
                &self.output,
                state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(self.output.clone()),
            );
        }
        Ok(())
    }
}

fn process_input(event: InputEvent<LibinputInputBackend>, state: &mut Villain) {
    match event {
        InputEvent::Keyboard { event } => crate::keybinds::handle_keyboard_event(state, event),
        InputEvent::PointerMotion { event } => {
            state.pointer_location += event.delta();
            state.pointer_location.x = state
                .pointer_location
                .x
                .clamp(0.0, f64::from(state.output_size.w - 1));
            state.pointer_location.y = state
                .pointer_location
                .y
                .clamp(0.0, f64::from(state.output_size.h - 1));
            state.refresh_pointer(event.time_msec());
        }
        InputEvent::PointerMotionAbsolute { event } => {
            state.pointer_location = (
                event.x_transformed(state.output_size.w),
                event.y_transformed(state.output_size.h),
            )
                .into();
            state.refresh_pointer(event.time_msec());
        }
        InputEvent::PointerButton { event } => {
            match event.state() {
                ButtonState::Pressed => {
                    state.pressed_buttons.insert(event.button_code());
                }
                ButtonState::Released => {
                    if !state.pressed_buttons.remove(&event.button_code()) {
                        return;
                    }
                }
            }
            state.refresh_pointer(event.time_msec());
            let pointer = state.pointer.clone();
            pointer.button(
                state,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    button: event.button_code(),
                    state: event.state(),
                },
            );
            pointer.frame(state);
        }
        InputEvent::PointerAxis { event } => {
            let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
            for axis in [Axis::Horizontal, Axis::Vertical] {
                let amount = event
                    .amount(axis)
                    .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
                frame = frame
                    .value(axis, amount)
                    .relative_direction(axis, event.relative_direction(axis));
                if let Some(steps) = event.amount_v120(axis) {
                    frame = frame.v120(axis, steps as i32);
                }
                if event.source() == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                    frame = frame.stop(axis);
                }
            }
            let pointer = state.pointer.clone();
            pointer.axis(state, frame);
            pointer.frame(state);
        }
        _ => {}
    }
}
