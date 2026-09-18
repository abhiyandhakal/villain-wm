//! Client-initiated move/resize grabs for floating windows.
use crate::{state::Villain, workspaces::constrain_geometry};
use smithay::{
    desktop::Window,
    input::pointer::*,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
    wayland::seat::WaylandFocus,
    xwayland::xwm::ResizeEdge,
};

type Geometry = Rectangle<i32, Logical>;
type PointerFocus = Option<(WlSurface, Point<f64, Logical>)>;

#[derive(Clone, Copy, Debug)]
pub struct ResizeEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}
impl ResizeEdges {
    pub fn from_xdg(edge: xdg_toplevel::ResizeEdge) -> Option<Self> {
        use xdg_toplevel::ResizeEdge::*;
        Some(match edge {
            Top => Self::from_x11(ResizeEdge::Top),
            Bottom => Self::from_x11(ResizeEdge::Bottom),
            Left => Self::from_x11(ResizeEdge::Left),
            Right => Self::from_x11(ResizeEdge::Right),
            TopLeft => Self::from_x11(ResizeEdge::TopLeft),
            TopRight => Self::from_x11(ResizeEdge::TopRight),
            BottomLeft => Self::from_x11(ResizeEdge::BottomLeft),
            BottomRight => Self::from_x11(ResizeEdge::BottomRight),
            _ => return Option::None,
        })
    }
    pub fn from_x11(edge: ResizeEdge) -> Self {
        use ResizeEdge::*;
        Self {
            left: matches!(edge, Left | TopLeft | BottomLeft),
            right: matches!(edge, Right | TopRight | BottomRight),
            top: matches!(edge, Top | TopLeft | TopRight),
            bottom: matches!(edge, Bottom | BottomLeft | BottomRight),
        }
    }
}

impl Villain {
    pub fn x11_grab_button_matches(&self, button: u32) -> bool {
        let code = match button {
            1 => 0x110,
            2 => 0x112,
            3 => 0x111,
            _ => return false,
        };
        self.pointer
            .grab_start_data()
            .is_some_and(|start| start.button == code && self.pressed_buttons.contains(&code))
    }

    pub fn start_window_grab(
        &mut self,
        window: Window,
        serial: Option<Serial>,
        edges: Option<ResizeEdges>,
    ) {
        if serial.is_some_and(|serial| !self.pointer.has_grab(serial))
            || self.space.element_location(&window).is_none()
        {
            return;
        }
        let Some(geometry) = self.floating_geometry(&window) else {
            return;
        };
        let Some(start) = self.pointer.grab_start_data() else {
            return;
        };
        if !self.pressed_buttons.contains(&start.button) {
            return;
        }
        let root = start.focus.as_ref().map(|(surface, _)| {
            std::iter::successors(
                Some(surface.clone()),
                smithay::wayland::compositor::get_parent,
            )
            .last()
            .unwrap()
        });
        if root != window.wl_surface().map(|surface| surface.into_owned()) {
            return;
        }
        if let Some(surface) = window.toplevel()
            && edges.is_some()
        {
            surface.with_pending_state(|state| state.states.set(xdg_toplevel::State::Resizing));
            surface.send_pending_configure();
        }
        self.pointer.clone().set_grab(
            self,
            WindowGrab {
                start,
                window,
                geometry,
                edges,
            },
            serial.unwrap_or_else(|| SERIAL_COUNTER.next_serial()),
            Focus::Clear,
        );
    }
}

struct WindowGrab {
    start: GrabStartData<Villain>,
    window: Window,
    geometry: Geometry,
    edges: Option<ResizeEdges>,
}

fn resize_geometry(
    start: Geometry,
    delta: Point<i32, Logical>,
    edges: ResizeEdges,
    output: smithay::utils::Size<i32, Logical>,
    min: smithay::utils::Size<i32, Logical>,
    max: smithay::utils::Size<i32, Logical>,
) -> Geometry {
    let mut rect = start;
    if edges.left {
        rect.size.w = start.size.w.saturating_sub(delta.x);
    }
    if edges.right {
        rect.size.w = start.size.w.saturating_add(delta.x);
    }
    if edges.top {
        rect.size.h = start.size.h.saturating_sub(delta.y);
    }
    if edges.bottom {
        rect.size.h = start.size.h.saturating_add(delta.y);
    }
    rect = constrain_geometry(rect, output, min, max);
    if edges.left {
        rect.loc.x = start.loc.x + start.size.w - rect.size.w;
    }
    if edges.top {
        rect.loc.y = start.loc.y + start.size.h - rect.size.h;
    }
    constrain_geometry(rect, output, min, max)
}

impl PointerGrab<Villain> for WindowGrab {
    fn motion(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        _focus: PointerFocus,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        if data.space.element_location(&self.window).is_none() {
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        }
        let delta = (event.location - self.start.location).to_i32_round();
        let geometry = if let Some(edges) = self.edges {
            let (min, max) = Villain::window_constraints(&self.window);
            resize_geometry(self.geometry, delta, edges, data.output_size, min, max)
        } else {
            Rectangle::new(self.geometry.loc + delta, self.geometry.size)
        };
        data.set_floating_geometry(&self.window, geometry);
    }
    fn relative_motion(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        focus: PointerFocus,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }
    fn button(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }
    fn axis(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }
    fn frame(&mut self, data: &mut Villain, handle: &mut PointerInnerHandle<'_, Villain>) {
        handle.frame(data);
    }
    fn start_data(&self) -> &GrabStartData<Villain> {
        &self.start
    }
    fn unset(&mut self, _data: &mut Villain) {
        if self.edges.is_some()
            && let Some(surface) = self.window.toplevel()
        {
            surface.with_pending_state(|state| state.states.unset(xdg_toplevel::State::Resizing));
            surface.send_pending_configure();
        }
    }
    fn gesture_swipe_begin(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }
    fn gesture_swipe_update(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }
    fn gesture_swipe_end(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }
    fn gesture_pinch_begin(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }
    fn gesture_pinch_update(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }
    fn gesture_pinch_end(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }
    fn gesture_hold_begin(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }
    fn gesture_hold_end(
        &mut self,
        data: &mut Villain,
        handle: &mut PointerInnerHandle<'_, Villain>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resizing_from_top_left_preserves_opposite_corner_at_minimum() {
        let initial = Rectangle::new((100, 100).into(), (300, 200).into());
        let rect = resize_geometry(
            initial,
            (500, 500).into(),
            ResizeEdges::from_x11(ResizeEdge::TopLeft),
            (800, 600).into(),
            (120, 80).into(),
            (0, 0).into(),
        );
        assert_eq!(rect, Rectangle::new((280, 220).into(), (120, 80).into()));
    }
    #[test]
    fn fixed_size_and_output_limits_survive_resize_requests() {
        let initial = Rectangle::new((10, 10).into(), (240, 180).into());
        let rect = resize_geometry(
            initial,
            (5000, 5000).into(),
            ResizeEdges::from_x11(ResizeEdge::BottomRight),
            (800, 600).into(),
            (240, 180).into(),
            (240, 180).into(),
        );
        assert_eq!(rect, initial);
        assert_eq!(
            constrain_geometry(
                Rectangle::new((-100, 5000).into(), (2000, 2000).into()),
                (800, 600).into(),
                (0, 0).into(),
                (0, 0).into()
            ),
            Rectangle::from_size((800, 600).into())
        );
    }
}
