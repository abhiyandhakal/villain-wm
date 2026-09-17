//! Compositor shortcuts are consumed; ordinary keys reach the active app.
use crate::state::Villain;
use smithay::{
    backend::input::{InputBackend, KeyState, KeyboardKeyEvent},
    input::keyboard::{FilterResult, keysyms},
    utils::SERIAL_COUNTER,
};

#[derive(Debug, PartialEq)]
enum Action {
    Close,
    Minimize,
    RestoreMinimized,
    Terminal,
    Workspace(usize),
    Quit,
    Vt(i32),
}

fn binding(sym: u32, active: usize, shift: bool) -> Option<Action> {
    match sym {
        keysyms::KEY_q if !shift => Some(Action::Close),
        keysyms::KEY_m if !shift => Some(Action::Minimize),
        keysyms::KEY_m if shift => Some(Action::RestoreMinimized),
        _ if shift => None,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(Action::Terminal),
        keysyms::KEY_1..=keysyms::KEY_9 => Some(Action::Workspace((sym - keysyms::KEY_1) as usize)),
        keysyms::KEY_0 => Some(Action::Workspace(9)),
        keysyms::KEY_Left => Some(Action::Workspace((active + 9) % 10)),
        keysyms::KEY_Right => Some(Action::Workspace((active + 1) % 10)),
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
                    Some(Action::Quit)
                } else if state.tty.is_some()
                    && (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&sym)
                {
                    Some(Action::Vt((sym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32))
                } else {
                    key.raw_syms().iter().find_map(|sym| {
                        (state.tty.is_some()
                            && (keysyms::KEY_F1..=keysyms::KEY_F12).contains(&sym.raw()))
                        .then(|| Action::Vt((sym.raw() - keysyms::KEY_F1 + 1) as i32))
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
                return FilterResult::Intercept(Some(action));
            }
            FilterResult::Forward
        },
    );
    match action.flatten() {
        Some(Action::Close) => state.close_focused_window(),
        Some(Action::Minimize) => state.minimize_focused_window(),
        Some(Action::RestoreMinimized) => state.restore_last_minimized_window(),
        Some(Action::Terminal) => state.launch_terminal(),
        Some(Action::Workspace(index)) => state.switch_workspace(index),
        Some(Action::Quit) => state.loop_signal.stop(),
        Some(Action::Vt(vt)) => {
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
            Some(Action::Workspace(0))
        );
        assert_eq!(
            binding(keysyms::KEY_0, 5, false),
            Some(Action::Workspace(9))
        );
        assert_eq!(
            binding(keysyms::KEY_Left, 0, false),
            Some(Action::Workspace(9))
        );
        assert_eq!(
            binding(keysyms::KEY_Right, 9, false),
            Some(Action::Workspace(0))
        );
        assert_eq!(
            binding(keysyms::KEY_Return, 0, false),
            Some(Action::Terminal)
        );
        assert_eq!(binding(keysyms::KEY_q, 0, false), Some(Action::Close));
        assert_eq!(binding(keysyms::KEY_m, 0, false), Some(Action::Minimize));
        assert_eq!(
            binding(keysyms::KEY_m, 0, true),
            Some(Action::RestoreMinimized)
        );
        assert_eq!(binding(keysyms::KEY_1, 0, true), None);
        assert_eq!(binding(keysyms::KEY_t, 0, false), None);
    }
}
