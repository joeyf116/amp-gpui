//! The keyboard. Bindings live here so there's one place to see them all, and
//! the docs' key list can be checked against it.

use gpui_kit::*;

actions!(
    rig,
    [
        Undo,
        Redo,
        RemoveUnit,
        BypassUnit,
        OpenUnit,
        CloseUnit,
        PreviousUnit,
        NextUnit,
        MoveUnitLeft,
        MoveUnitRight,
        PreviousTone,
        NextTone,
        ToggleMonitoring,
        ToggleTools,
        ToggleMetronome,
        ToggleTones,
        ToggleGear,
        ToggleAssistant,
        CompareTrial,
        TonePrevious,
        ToneNext,
        SaveTone,
        Scene1,
        Scene2,
        Scene3,
        Scene4
    ]
);

/// Single letters and digits that act on the rig. Text fields sit inside the
/// rig, so these are switched off in them, or typing a tone's name would bypass
/// a pedal or change scene.
const LETTER_KEYS: [&str; 9] = ["b", "g", "k", "m", "t", "1", "2", "3", "4"];

/// Keys that act on the board.
pub fn bind_keys(cx: &mut App) {
    let rig = Some("Rig");
    cx.bind_keys([
        KeyBinding::new("ctrl-z", Undo, rig),
        KeyBinding::new("ctrl-y", Redo, rig),
        KeyBinding::new("ctrl-shift-z", Redo, rig),
        KeyBinding::new("delete", RemoveUnit, rig),
        KeyBinding::new("b", BypassUnit, rig),
        KeyBinding::new("enter", OpenUnit, rig),
        KeyBinding::new("escape", CloseUnit, rig),
        KeyBinding::new("left", PreviousUnit, rig),
        KeyBinding::new("right", NextUnit, rig),
        KeyBinding::new("alt-left", MoveUnitLeft, rig),
        KeyBinding::new("alt-right", MoveUnitRight, rig),
        KeyBinding::new("pageup", PreviousTone, rig),
        KeyBinding::new("pagedown", NextTone, rig),
        KeyBinding::new("m", ToggleMonitoring, rig),
        KeyBinding::new("t", ToggleTools, rig),
        KeyBinding::new("k", ToggleMetronome, rig),
        KeyBinding::new("up", TonePrevious, rig),
        KeyBinding::new("down", ToneNext, rig),
        KeyBinding::new("ctrl-o", ToggleTones, rig),
        KeyBinding::new("g", ToggleGear, rig),
        KeyBinding::new("ctrl-j", ToggleAssistant, rig),
        KeyBinding::new("ctrl-b", CompareTrial, rig),
        KeyBinding::new("ctrl-s", SaveTone, rig),
        KeyBinding::new("1", Scene1, rig),
        KeyBinding::new("2", Scene2, rig),
        KeyBinding::new("3", Scene3, rig),
        KeyBinding::new("4", Scene4, rig),
    ]);
    cx.bind_keys(
        LETTER_KEYS
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some("Input"))),
    );
}
