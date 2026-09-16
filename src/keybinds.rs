//! Compositor shortcuts are consumed; ordinary keys reach the active app.
use crate::state::Villain;
use smithay::{
    backend::{
        input::{Event, KeyState, KeyboardKeyEvent},
        winit::WinitKeyboardInputEvent,
    },
    input::keyboard::{FilterResult, keysyms},
    utils::SERIAL_COUNTER,
};

#[derive(Debug, PartialEq)]
enum Action {
    Terminal,
    Workspace(usize),
}

fn binding(sym: u32, active: usize) -> Option<Action> {
    match sym {
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(Action::Terminal),
        keysyms::KEY_1..=keysyms::KEY_9 => Some(Action::Workspace((sym - keysyms::KEY_1) as usize)),
        keysyms::KEY_0 => Some(Action::Workspace(9)),
        keysyms::KEY_Left => Some(Action::Workspace((active + 9) % 10)),
        keysyms::KEY_Right => Some(Action::Workspace((active + 1) % 10)),
        _ => None,
    }
}

pub fn handle_keyboard_event(state: &mut Villain, event: WinitKeyboardInputEvent) {
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
            if pressed
                && mods.alt
                && !mods.ctrl
                && !mods.logo
                && !mods.shift
                && let Some(action) = key
                    .raw_syms()
                    .iter()
                    .find_map(|sym| binding(sym.raw(), state.active_workspace))
            {
                state.suppressed_keys.insert(code);
                return FilterResult::Intercept(Some(action));
            }
            FilterResult::Forward
        },
    );
    match action.flatten() {
        Some(Action::Terminal) => state.launch_terminal(),
        Some(Action::Workspace(index)) => state.switch_workspace(index),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_numbers_and_wraparound() {
        assert_eq!(binding(keysyms::KEY_1, 5), Some(Action::Workspace(0)));
        assert_eq!(binding(keysyms::KEY_0, 5), Some(Action::Workspace(9)));
        assert_eq!(binding(keysyms::KEY_Left, 0), Some(Action::Workspace(9)));
        assert_eq!(binding(keysyms::KEY_Right, 9), Some(Action::Workspace(0)));
        assert_eq!(binding(keysyms::KEY_Return, 0), Some(Action::Terminal));
        assert_eq!(binding(keysyms::KEY_t, 0), None);
    }
}
