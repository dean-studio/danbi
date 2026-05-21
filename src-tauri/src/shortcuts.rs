use crate::error::{DanbiError, DanbiResult};
use std::sync::Mutex;
use tauri::Runtime;
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcut, Modifiers, Shortcut,
};

/// Remember the currently registered capture shortcut so we can unregister it
/// cleanly before applying a new one.
static CURRENT: Mutex<Option<Shortcut>> = Mutex::new(None);

/// Parses an Electron-style accelerator string into Tauri's Shortcut type.
/// Supported tokens:
///   CommandOrControl, Command, Super, Control, Shift, Option, Alt, Meta
///   Keys: A-Z, 0-9, F1-F24, Space, Enter/Return, Escape/Esc, Tab,
///         Backspace/BackSpace, Delete/Del, ArrowUp/Down/Left/Right,
///         Up/Down/Left/Right, punctuation (`,`, `.`, `/`, `;`, `'`, `[`, `]`,
///         `\\`, `` ` ``, `-`, `=`).
fn parse(accelerator: &str) -> DanbiResult<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;

    for raw in accelerator.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" => {
                #[cfg(target_os = "macos")]
                {
                    mods |= Modifiers::SUPER;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    mods |= Modifiers::CONTROL;
                }
            }
            "command" | "cmd" | "super" | "meta" => mods |= Modifiers::SUPER,
            "control" | "ctrl" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" | "opt" => mods |= Modifiers::ALT,
            other => {
                key = Some(parse_key(other).ok_or_else(|| {
                    DanbiError::Config(format!("unknown key: {other}"))
                })?);
            }
        }
    }

    let code = key.ok_or_else(|| {
        DanbiError::Config(format!(
            "accelerator '{accelerator}' has no key (e.g. 'Control+Space')"
        ))
    })?;

    let mods_opt = if mods.is_empty() { None } else { Some(mods) };
    Ok(Shortcut::new(mods_opt, code))
}

fn parse_key(token: &str) -> Option<Code> {
    let lower = token.to_ascii_lowercase();
    Some(match lower.as_str() {
        // Letters
        "a" => Code::KeyA,
        "b" => Code::KeyB,
        "c" => Code::KeyC,
        "d" => Code::KeyD,
        "e" => Code::KeyE,
        "f" => Code::KeyF,
        "g" => Code::KeyG,
        "h" => Code::KeyH,
        "i" => Code::KeyI,
        "j" => Code::KeyJ,
        "k" => Code::KeyK,
        "l" => Code::KeyL,
        "m" => Code::KeyM,
        "n" => Code::KeyN,
        "o" => Code::KeyO,
        "p" => Code::KeyP,
        "q" => Code::KeyQ,
        "r" => Code::KeyR,
        "s" => Code::KeyS,
        "t" => Code::KeyT,
        "u" => Code::KeyU,
        "v" => Code::KeyV,
        "w" => Code::KeyW,
        "x" => Code::KeyX,
        "y" => Code::KeyY,
        "z" => Code::KeyZ,
        // Digits
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        // Function keys
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        "f13" => Code::F13,
        "f14" => Code::F14,
        "f15" => Code::F15,
        "f16" => Code::F16,
        "f17" => Code::F17,
        "f18" => Code::F18,
        "f19" => Code::F19,
        "f20" => Code::F20,
        // Named
        "space" | "spacebar" => Code::Space,
        "enter" | "return" => Code::Enter,
        "escape" | "esc" => Code::Escape,
        "tab" => Code::Tab,
        "backspace" | "back" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "up" | "arrowup" => Code::ArrowUp,
        "down" | "arrowdown" => Code::ArrowDown,
        "left" | "arrowleft" => Code::ArrowLeft,
        "right" | "arrowright" => Code::ArrowRight,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "," | "comma" => Code::Comma,
        "." | "period" => Code::Period,
        "/" | "slash" => Code::Slash,
        ";" | "semicolon" => Code::Semicolon,
        "'" | "quote" => Code::Quote,
        "[" | "bracketleft" => Code::BracketLeft,
        "]" | "bracketright" => Code::BracketRight,
        "\\" | "backslash" => Code::Backslash,
        "`" | "backquote" => Code::Backquote,
        "-" | "minus" => Code::Minus,
        "=" | "equal" => Code::Equal,
        _ => return None,
    })
}

/// Registers the capture shortcut, replacing any previously registered one.
/// 빈 문자열을 넘기면 비활성화로 해석 — 이전 등록만 해제하고 종료한다.
pub fn apply_capture_shortcut<R: Runtime>(
    gs: &GlobalShortcut<R>,
    accelerator: &str,
) -> DanbiResult<()> {
    let trimmed = accelerator.trim();
    let mut slot = CURRENT
        .lock()
        .map_err(|_| DanbiError::Other("shortcut lock".into()))?;
    if let Some(prev) = slot.take() {
        let _ = gs.unregister(prev);
    }
    if trimmed.is_empty() {
        return Ok(());
    }

    let shortcut = parse(trimmed)?;
    gs.register(shortcut)
        .map_err(|e| DanbiError::Other(format!("register {trimmed}: {e}")))?;
    *slot = Some(shortcut);
    Ok(())
}

/// Helper for the settings UI: validates an accelerator string without
/// registering. Useful to give immediate feedback as the user types.
pub fn validate_accelerator(accelerator: &str) -> DanbiResult<()> {
    parse(accelerator).map(|_| ())
}
