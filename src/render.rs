//! The nested Winit backend and the one-frame render path.

use std::time::Duration;

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, Event, InputEvent, KeyState, PointerAxisEvent,
            PointerButtonEvent,
        },
        renderer::{
            damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::space::render_output,
    input::pointer::{AxisFrame, ButtonEvent},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::state::Villain;

/// Start a nested output and connect its events to the compositor loop.
pub fn init_winit(
    event_loop: &mut EventLoop<Villain>,
    state: &mut Villain,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit_source) = winit::init()?;
    backend.window().set_cursor_visible(true);
    state.output_size = backend.window_size().to_logical(1);
    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "villain-output".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Villain".into(),
            model: "Nested Winit output".into(),
        },
    );
    let _global = output.create_global::<Villain>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    event_loop
        .handle()
        .insert_source(winit_source, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                state.output_size = size.to_logical(1);
                for window in state.workspaces.iter().flatten() {
                    state.configure_window(window);
                }
                output.change_current_state(
                    Some(Mode {
                        size,
                        refresh: 60_000,
                    }),
                    None,
                    None,
                    None,
                );
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                crate::keybinds::handle_keyboard_event(state, event);
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
                state.pointer_location = (event.x(), event.y()).into();
                state.refresh_pointer(event.time_msec());
            }
            WinitEvent::Input(InputEvent::PointerButton { event }) => {
                match event.state() {
                    smithay::backend::input::ButtonState::Pressed => {
                        state.pressed_buttons.insert(event.button_code());
                    }
                    smithay::backend::input::ButtonState::Released => {
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
                        serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        button: event.button_code(),
                        state: event.state(),
                    },
                );
                pointer.frame(state);
            }
            WinitEvent::Input(InputEvent::PointerAxis { event }) => {
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
            WinitEvent::Focus(focused) => {
                state.host_focused = focused;
                if !focused {
                    state.release_pointer_buttons();
                    let keyboard = state.keyboard.clone();
                    for code in keyboard.pressed_keys() {
                        keyboard.input::<(), _>(
                            state,
                            code,
                            KeyState::Released,
                            smithay::utils::SERIAL_COUNTER.next_serial(),
                            0,
                            |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                        );
                    }
                    state.suppressed_keys.clear();
                }
                state.focus_active();
            }
            WinitEvent::Redraw => {
                let size = backend.window_size();
                let damage = Rectangle::from_size(size);
                let (renderer, mut framebuffer) = backend.bind().expect("bind Winit framebuffer");

                render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
                    &output,
                    renderer,
                    &mut framebuffer,
                    1.0,
                    0,
                    [&state.space],
                    &[],
                    &mut damage_tracker,
                    [0.08, 0.05, 0.12, 1.0],
                )
                .expect("render Villain output");

                drop(framebuffer);
                backend
                    .submit(Some(&[damage]))
                    .expect("submit Villain frame");

                state.space.elements().for_each(|window| {
                    window.send_frame(
                        &output,
                        state.start_time.elapsed(),
                        Some(Duration::ZERO),
                        |_, _| Some(output.clone()),
                    );
                });
                state.space.refresh();
                let _ = state.display_handle.flush_clients();
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })?;

    Ok(())
}
