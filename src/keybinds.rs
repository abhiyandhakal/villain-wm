//! Compositor shortcuts are consumed; ordinary keys reach the active app.
use crate::{dispatch::Dispatch, state::Villain};
use smithay::{
    backend::input::{InputBackend, KeyState, KeyboardKeyEvent},
    input::keyboard::{FilterResult, keysyms},
    utils::SERIAL_COUNTER,
};

#[derive(Debug, PartialEq)]
enum KeyboardAction {
    Dispatch(Dispatch),
    Vt(i32),
}

fn binding(sym: u32, active: usize, shift: bool) -> Option<Dispatch> {
    match sym {
        keysyms::KEY_q if !shift => Some(Dispatch::CloseFocused),
        keysyms::KEY_m if !shift => Some(Dispatch::MinimizeFocused),
        keysyms::KEY_m if shift => Some(Dispatch::RestoreLastMinimized),
        _ if shift => None,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(Dispatch::Spawn(vec![
            std::env::var("VILLAIN_TERMINAL").unwrap_or_else(|_| "kitty".into()),
        ])),
        keysyms::KEY_1..=keysyms::KEY_9 => Some(Dispatch::FocusWorkspace(
            (sym - keysyms::KEY_1) as usize + 1,
        )),
        keysyms::KEY_0 => Some(Dispatch::FocusWorkspace(10)),
        keysyms::KEY_Left => Some(Dispatch::FocusWorkspace((active + 9) % 10 + 1)),
        keysyms::KEY_Right => Some(Dispatch::FocusWorkspace((active + 1) % 10 + 1)),
        _ => None,
    }
}

pub fn handle_keyboard_event<B: InputBackend>(
    state: &mut Villain,
    event: impl KeyboardKeyEvent<B>,
) {
    let code = event.key_code();
    let pressed = event.state() == KeyState::Pressed;
    let keyboard = state.keyboard.clone();
    let action = keyboard.input(
        state,
        code,
        event.state(),
        SERIAL_COUNTER.next_serial(),
        event.time_msec(),
        |state, mods, key| {
            if !pressed && state.suppressed_keys.remove(&code) {
                return FilterResult::Intercept(None);
            }
            if state.suppressed_keys.contains(&code) {
                return FilterResult::Intercept(None);
            }
            if pressed {
                tracing::debug!(?code, sym = ?key.modified_sym(), "key pressed");
            }
            if pressed && mods.ctrl && mods.alt {
                let sym = key.modified_sym().raw();
                let action = if key
                    .raw_syms()
                    .iter()
                    .any(|sym| sym.raw() == keysyms::KEY_BackSpace)
                {
                    Some(KeyboardAction::Dispatch(Dispatch::Quit))
                } else if state.tty.is_some()
                    && (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&sym)
                {
                    Some(KeyboardAction::Vt(
                        (sym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32,
                    ))
                } else {
                    key.raw_syms().iter().find_map(|sym| {
                        (state.tty.is_some()
                            && (keysyms::KEY_F1..=keysyms::KEY_F12).contains(&sym.raw()))
                        .then(|| KeyboardAction::Vt((sym.raw() - keysyms::KEY_F1 + 1) as i32))
                    })
                };
                if let Some(action) = action {
                    state.suppressed_keys.insert(code);
                    return FilterResult::Intercept(Some(action));
                }
            }
            if pressed
                && mods.alt
                && !mods.ctrl
                && !mods.logo
                && let Some(action) = key
                    .raw_syms()
                    .iter()
                    .find_map(|sym| binding(sym.raw(), state.active_workspace, mods.shift))
            {
                state.suppressed_keys.insert(code);
                return FilterResult::Intercept(Some(KeyboardAction::Dispatch(action)));
            }
            FilterResult::Forward
        },
    );
    match action.flatten() {
        Some(KeyboardAction::Dispatch(dispatch)) => {
            if let Err(error) = state.dispatch(dispatch) {
                tracing::debug!(%error, "keybind dispatch had no effect");
            }
        }
        Some(KeyboardAction::Vt(vt)) => {
            use smithay::backend::session::Session;
            if let Some(tty) = state.tty.as_mut()
                && let Err(error) = tty.session.change_vt(vt)
            {
                tracing::warn!(%error, "VT switch failed");
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_numbers_and_wraparound() {
        assert_eq!(
            binding(keysyms::KEY_1, 5, false),
            Some(Dispatch::FocusWorkspace(1))
        );
        assert_eq!(
            binding(keysyms::KEY_0, 5, false),
            Some(Dispatch::FocusWorkspace(10))
        );
        assert_eq!(
            binding(keysyms::KEY_Left, 0, false),
            Some(Dispatch::FocusWorkspace(10))
        );
        assert_eq!(
            binding(keysyms::KEY_Right, 9, false),
            Some(Dispatch::FocusWorkspace(1))
        );
        assert_eq!(
            binding(keysyms::KEY_Return, 0, false),
            Some(Dispatch::Spawn(vec![
                std::env::var("VILLAIN_TERMINAL").unwrap_or_else(|_| "kitty".into())
            ]))
        );
        assert_eq!(
            binding(keysyms::KEY_q, 0, false),
            Some(Dispatch::CloseFocused)
        );
        assert_eq!(
            binding(keysyms::KEY_m, 0, false),
            Some(Dispatch::MinimizeFocused)
        );
        assert_eq!(
            binding(keysyms::KEY_m, 0, true),
            Some(Dispatch::RestoreLastMinimized)
        );
        assert_eq!(binding(keysyms::KEY_1, 0, true), None);
        assert_eq!(binding(keysyms::KEY_t, 0, false), None);
    }
}
