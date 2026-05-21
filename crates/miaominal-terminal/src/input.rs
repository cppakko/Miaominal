//! Pure terminal-domain helpers shared by the UI shell and terminal state.
//!
//! This module intentionally avoids GPUI window/render dependencies and keeps
//! terminal value types plus keyboard/paste helpers in one place.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchMatchKind {
    #[default]
    None,
    Match,
    Current,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalInputModes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub focus_in_out: bool,
    pub kitty_keyboard_protocol: bool,
    pub kitty_disambiguate_escape_codes: bool,
    pub kitty_report_event_types: bool,
    pub kitty_report_alternate_keys: bool,
    pub kitty_report_all_keys_as_escape_codes: bool,
    pub kitty_report_associated_text: bool,
}

impl TerminalInputModes {
    pub fn kitty_sequence_enabled(self) -> bool {
        self.kitty_report_all_keys_as_escape_codes
            || self.kitty_disambiguate_escape_codes
            || self.kitty_report_event_types
    }
}

pub fn sanitize_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let mut sanitized = String::with_capacity(text.len());
    let mut prev_cr = false;
    for ch in text.chars() {
        match ch {
            '\r' => {
                sanitized.push('\r');
                prev_cr = true;
                continue;
            }
            '\n' => {
                if !prev_cr {
                    sanitized.push('\r');
                }
            }
            '\t' => sanitized.push('\t'),
            c if (c as u32) < 0x20 => {}
            c if (c as u32) == 0x7f => {}
            c if (0x80..=0x9f).contains(&(c as u32)) => {}
            c => sanitized.push(c),
        }
        prev_cr = false;
    }

    if bracketed {
        let mut buffer = Vec::with_capacity(sanitized.len() + 12);
        buffer.extend_from_slice(b"\x1b[200~");
        buffer.extend_from_slice(sanitized.as_bytes());
        buffer.extend_from_slice(b"\x1b[201~");
        buffer
    } else {
        sanitized.into_bytes()
    }
}

/// Keyboard input encoding stays UI-agnostic: it only depends on `gpui`
/// keystroke types plus terminal input mode values.
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKeyPhase {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalKeyEvent<'a> {
    pub keystroke: &'a gpui::Keystroke,
    pub phase: TerminalKeyPhase,
}

impl<'a> TerminalKeyEvent<'a> {
    pub fn new(keystroke: &'a gpui::Keystroke, phase: TerminalKeyPhase) -> Self {
        Self { keystroke, phase }
    }
}

pub fn encode_terminal_input(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    if event.phase == TerminalKeyPhase::Release && !input_modes.kitty_report_event_types {
        return None;
    }

    if let Some(bytes) = encode_application_keypad(event, input_modes) {
        return Some(bytes);
    }

    if let Some(bytes) = encode_legacy_binding(event, input_modes) {
        return Some(bytes);
    }

    if should_build_kitty_sequence(event, input_modes)
        && let Some(bytes) = encode_kitty_input(event, input_modes)
    {
        return Some(bytes);
    }

    if let Some(bytes) = encode_named_key_normal(event.keystroke, input_modes) {
        return Some(bytes);
    }

    encode_text_input(event.keystroke)
}

fn control_sequence(key: &str) -> Option<u8> {
    if key.eq_ignore_ascii_case("space") {
        return Some(0);
    }

    let character = key.chars().next()?;
    Some(match character {
        'a'..='z' => character as u8 - b'a' + 1,
        'A'..='Z' => character as u8 - b'A' + 1,
        '@' | '2' | ' ' => 0,
        '[' | '3' => 27,
        '\\' | '4' => 28,
        ']' | '5' => 29,
        '^' | '6' => 30,
        '_' | '7' | '/' => 31,
        '8' | '?' => 127,
        _ => return None,
    })
}

fn encode_text_input(keystroke: &gpui::Keystroke) -> Option<Vec<u8>> {
    let mut bytes = match keystroke.key.as_str() {
        "space" => {
            if keystroke.modifiers.control {
                vec![0]
            } else {
                vec![b' ']
            }
        }
        "enter" | "return" => vec![b'\r'],
        "tab" if keystroke.modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        "backspace" => vec![0x7f],
        "escape" => vec![0x1b],
        _ => {
            if keystroke.modifiers.platform {
                return None;
            }

            if keystroke.modifiers.control {
                vec![control_sequence(&keystroke.key)?]
            } else if let Some(key_char) = &keystroke.key_char {
                key_char.as_bytes().to_vec()
            } else if keystroke.key.len() == 1 {
                keystroke.key.as_bytes().to_vec()
            } else {
                return None;
            }
        }
    };

    if keystroke.modifiers.alt {
        let mut prefixed = vec![0x1b];
        prefixed.append(&mut bytes);
        Some(prefixed)
    } else {
        Some(bytes)
    }
}

fn encode_named_key_normal(
    keystroke: &gpui::Keystroke,
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    if keystroke.modifiers.platform {
        return None;
    }

    let modifiers = sequence_modifiers(keystroke.modifiers);
    let key = keystroke.key.as_str();
    let (base, terminator, uses_ss3, is_functional) = match key {
        "pageup" => (
            Cow::Borrowed("5"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "pagedown" => (
            Cow::Borrowed("6"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "insert" => (
            Cow::Borrowed("2"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "delete" => (
            Cow::Borrowed("3"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "home" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('H'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "end" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('F'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "left" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('D'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "right" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('C'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "up" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('A'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "down" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('B'),
            input_modes.app_cursor && modifiers.is_empty(),
            true,
        ),
        "f1" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('P'),
            modifiers.is_empty(),
            true,
        ),
        "f2" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('Q'),
            modifiers.is_empty(),
            true,
        ),
        "f3" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('R'),
            modifiers.is_empty(),
            true,
        ),
        "f4" => (
            if modifiers.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Borrowed("1")
            },
            SequenceTerminator::Normal('S'),
            modifiers.is_empty(),
            true,
        ),
        "f5" => (
            Cow::Borrowed("15"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f6" => (
            Cow::Borrowed("17"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f7" => (
            Cow::Borrowed("18"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f8" => (
            Cow::Borrowed("19"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f9" => (
            Cow::Borrowed("20"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f10" => (
            Cow::Borrowed("21"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f11" => (
            Cow::Borrowed("23"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        "f12" => (
            Cow::Borrowed("24"),
            SequenceTerminator::Normal('~'),
            false,
            true,
        ),
        _ => return None,
    };

    if key == "tab"
        && keystroke.modifiers.shift
        && !keystroke.modifiers.alt
        && !keystroke.modifiers.control
    {
        return Some(b"\x1b[Z".to_vec());
    }

    if !is_functional {
        return None;
    }

    let modifier_parameter = modifiers.encode_esc_sequence();
    Some(match terminator {
        SequenceTerminator::Normal(final_char) if uses_ss3 => {
            format!("\x1bO{final_char}").into_bytes()
        }
        SequenceTerminator::Normal('~') if modifiers.is_empty() => {
            format!("\x1b[{base}~").into_bytes()
        }
        SequenceTerminator::Normal(final_char) if modifiers.is_empty() => {
            format!("\x1b[{base}{final_char}").into_bytes()
        }
        SequenceTerminator::Normal('~') => {
            format!("\x1b[{base};{modifier_parameter}~").into_bytes()
        }
        SequenceTerminator::Normal(final_char) => {
            format!("\x1b[{base};{modifier_parameter}{final_char}").into_bytes()
        }
        SequenceTerminator::Kitty => return None,
    })
}

fn encode_legacy_binding(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    LEGACY_KEY_BINDINGS
        .iter()
        .copied()
        .find(|binding| binding.matches(event, input_modes))
        .map(|binding| binding.bytes.to_vec())
}

fn encode_application_keypad(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    if !input_modes.app_keypad || event.phase == TerminalKeyPhase::Release {
        return None;
    }

    let modifiers = event.keystroke.modifiers;
    if modifiers.shift || modifiers.alt || modifiers.control || modifiers.platform {
        return None;
    }

    let suffix = match event.keystroke.key.as_str() {
        "numpad0" | "kp0" | "kp_0" => 'p',
        "numpad1" | "kp1" | "kp_1" => 'q',
        "numpad2" | "kp2" | "kp_2" => 'r',
        "numpad3" | "kp3" | "kp_3" => 's',
        "numpad4" | "kp4" | "kp_4" => 't',
        "numpad5" | "kp5" | "kp_5" => 'u',
        "numpad6" | "kp6" | "kp_6" => 'v',
        "numpad7" | "kp7" | "kp_7" => 'w',
        "numpad8" | "kp8" | "kp_8" => 'x',
        "numpad9" | "kp9" | "kp_9" => 'y',
        "numpaddecimal" | "kpdecimal" | "kp_decimal" => 'n',
        "numpaddivide" | "kpdivide" | "kp_divide" => 'o',
        "numpadmultiply" | "kpmultiply" | "kp_multiply" => 'j',
        "numpadsubtract" | "kpsubtract" | "kp_subtract" => 'm',
        "numpadadd" | "kpadd" | "kp_add" => 'k',
        "numpadenter" | "kpenter" | "kp_enter" => 'M',
        "numpadequal" | "kpequal" | "kp_equal" => 'X',
        _ => return None,
    };

    Some(format!("\x1bO{suffix}").into_bytes())
}

fn should_build_kitty_sequence(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> bool {
    if !input_modes.kitty_keyboard_protocol {
        return false;
    }

    if input_modes.kitty_report_all_keys_as_escape_codes {
        return true;
    }

    let modifiers = event.keystroke.modifiers;
    let disambiguate = input_modes.kitty_disambiguate_escape_codes
        && (event.keystroke.key == "escape"
            || is_best_effort_keypad_key(event.keystroke.key.as_str())
            || ((modifiers.alt || modifiers.control || modifiers.platform)
                || (modifiers.shift
                    && matches!(event.keystroke.key.as_str(), "tab" | "enter" | "backspace"))));

    if disambiguate {
        return true;
    }

    if has_textual_alternate_key(event, input_modes) {
        return true;
    }

    if is_named_key_without_text(event.keystroke) {
        return true;
    }

    event
        .keystroke
        .key_char
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true)
}

fn encode_kitty_input(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    let mut modifiers = sequence_modifiers(event.keystroke.modifiers);
    if has_textual_alternate_key(event, input_modes) {
        modifiers.set(SequenceModifier::Shift, false);
    }
    let kitty_event_type = input_modes.kitty_report_event_types
        && matches!(
            event.phase,
            TerminalKeyPhase::Repeat | TerminalKeyPhase::Release
        );
    let associated_text = associated_text(event, input_modes);

    let sequence_base = try_build_kitty_numpad(event)
        .or_else(|| try_build_kitty_named_functional(event))
        .or_else(|| {
            try_build_kitty_named_normal(
                event,
                modifiers,
                kitty_event_type,
                associated_text.is_some(),
            )
        })
        .or_else(|| try_build_kitty_control_char_or_modifier(event, input_modes, &mut modifiers))
        .or_else(|| try_build_kitty_textual(event, input_modes, associated_text));

    let sequence_base = sequence_base?;
    let mut payload = format!("\x1b[{}", sequence_base.payload);

    if kitty_event_type || !modifiers.is_empty() || associated_text.is_some() {
        payload.push_str(&format!(";{}", modifiers.encode_esc_sequence()));
    }

    if kitty_event_type {
        payload.push(':');
        payload.push(match event.phase {
            TerminalKeyPhase::Press => '1',
            TerminalKeyPhase::Repeat => '2',
            TerminalKeyPhase::Release => '3',
        });
    }

    if let Some(text) = associated_text {
        let mut codepoints = text.chars().map(u32::from);
        if let Some(codepoint) = codepoints.next() {
            payload.push_str(&format!(";{codepoint}"));
        }
        for codepoint in codepoints {
            payload.push_str(&format!(":{codepoint}"));
        }
    }

    payload.push(sequence_base.terminator.encode_esc_sequence());
    Some(payload.into_bytes())
}

fn has_textual_alternate_key(event: TerminalKeyEvent<'_>, input_modes: TerminalInputModes) -> bool {
    input_modes.kitty_report_alternate_keys
        && event
            .keystroke
            .key_char
            .as_deref()
            .is_some_and(|character| {
                let mut characters = character.chars();
                let Some(actual_character) = characters.next() else {
                    return false;
                };
                if characters.next().is_some() {
                    return false;
                }
                base_text_key_char(event.keystroke)
                    .is_some_and(|base_character| actual_character != base_character)
            })
}

fn try_build_kitty_textual(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
    associated_text: Option<&str>,
) -> Option<SequenceBase> {
    if !input_modes.kitty_sequence_enabled() {
        return None;
    }

    let character = event.keystroke.key_char.as_deref()?;
    if character.chars().count() == 1 {
        let actual_char = character.chars().next()?;
        let base_char = base_text_key_char(event.keystroke)?;
        let payload = if input_modes.kitty_report_alternate_keys
            && u32::from(actual_char) != u32::from(base_char)
        {
            format!("{}:{}", u32::from(base_char), u32::from(actual_char))
        } else {
            u32::from(base_char).to_string()
        };

        Some(SequenceBase::new(payload, SequenceTerminator::Kitty))
    } else if input_modes.kitty_report_all_keys_as_escape_codes && associated_text.is_some() {
        Some(SequenceBase::new("0", SequenceTerminator::Kitty))
    } else {
        None
    }
}

fn try_build_kitty_numpad(event: TerminalKeyEvent<'_>) -> Option<SequenceBase> {
    let base = match event.keystroke.key.as_str() {
        "numpad0" | "kp0" | "kp_0" => "57399",
        "numpad1" | "kp1" | "kp_1" => "57400",
        "numpad2" | "kp2" | "kp_2" => "57401",
        "numpad3" | "kp3" | "kp_3" => "57402",
        "numpad4" | "kp4" | "kp_4" => "57403",
        "numpad5" | "kp5" | "kp_5" => "57404",
        "numpad6" | "kp6" | "kp_6" => "57405",
        "numpad7" | "kp7" | "kp_7" => "57406",
        "numpad8" | "kp8" | "kp_8" => "57407",
        "numpad9" | "kp9" | "kp_9" => "57408",
        "numpaddecimal" | "kpdecimal" | "kp_decimal" => "57409",
        "numpaddivide" | "kpdivide" | "kp_divide" => "57410",
        "numpadmultiply" | "kpmultiply" | "kp_multiply" => "57411",
        "numpadsubtract" | "kpsubtract" | "kp_subtract" => "57412",
        "numpadadd" | "kpadd" | "kp_add" => "57413",
        "numpadenter" | "kpenter" | "kp_enter" => "57414",
        "numpadequal" | "kpequal" | "kp_equal" => "57415",
        _ => return None,
    };

    Some(SequenceBase::new(base, SequenceTerminator::Kitty))
}

fn try_build_kitty_named_functional(event: TerminalKeyEvent<'_>) -> Option<SequenceBase> {
    let (base, terminator) = match event.keystroke.key.as_str() {
        "f3" => ("13", SequenceTerminator::Normal('~')),
        "f13" => ("57376", SequenceTerminator::Kitty),
        "f14" => ("57377", SequenceTerminator::Kitty),
        "f15" => ("57378", SequenceTerminator::Kitty),
        "f16" => ("57379", SequenceTerminator::Kitty),
        "f17" => ("57380", SequenceTerminator::Kitty),
        "f18" => ("57381", SequenceTerminator::Kitty),
        "f19" => ("57382", SequenceTerminator::Kitty),
        "f20" => ("57383", SequenceTerminator::Kitty),
        "f21" => ("57384", SequenceTerminator::Kitty),
        "f22" => ("57385", SequenceTerminator::Kitty),
        "f23" => ("57386", SequenceTerminator::Kitty),
        "f24" => ("57387", SequenceTerminator::Kitty),
        "f25" => ("57388", SequenceTerminator::Kitty),
        "f26" => ("57389", SequenceTerminator::Kitty),
        "f27" => ("57390", SequenceTerminator::Kitty),
        "f28" => ("57391", SequenceTerminator::Kitty),
        "f29" => ("57392", SequenceTerminator::Kitty),
        "f30" => ("57393", SequenceTerminator::Kitty),
        "f31" => ("57394", SequenceTerminator::Kitty),
        "f32" => ("57395", SequenceTerminator::Kitty),
        "f33" => ("57396", SequenceTerminator::Kitty),
        "f34" => ("57397", SequenceTerminator::Kitty),
        "f35" => ("57398", SequenceTerminator::Kitty),
        "scrolllock" => ("57359", SequenceTerminator::Kitty),
        "printscreen" => ("57361", SequenceTerminator::Kitty),
        "pause" => ("57362", SequenceTerminator::Kitty),
        "contextmenu" => ("57363", SequenceTerminator::Kitty),
        _ => return None,
    };

    Some(SequenceBase::new(base, terminator))
}

fn try_build_kitty_named_normal(
    event: TerminalKeyEvent<'_>,
    modifiers: SequenceModifiers,
    kitty_event_type: bool,
    has_associated_text: bool,
) -> Option<SequenceBase> {
    let one_based = if modifiers.is_empty() && !kitty_event_type && !has_associated_text {
        Cow::Borrowed("")
    } else {
        Cow::Borrowed("1")
    };

    let (base, terminator) = match event.keystroke.key.as_str() {
        "pageup" => (Cow::Borrowed("5"), SequenceTerminator::Normal('~')),
        "pagedown" => (Cow::Borrowed("6"), SequenceTerminator::Normal('~')),
        "insert" => (Cow::Borrowed("2"), SequenceTerminator::Normal('~')),
        "delete" => (Cow::Borrowed("3"), SequenceTerminator::Normal('~')),
        "home" => (one_based, SequenceTerminator::Normal('H')),
        "end" => (one_based, SequenceTerminator::Normal('F')),
        "left" => (one_based, SequenceTerminator::Normal('D')),
        "right" => (one_based, SequenceTerminator::Normal('C')),
        "up" => (one_based, SequenceTerminator::Normal('A')),
        "down" => (one_based, SequenceTerminator::Normal('B')),
        "f1" => (one_based, SequenceTerminator::Normal('P')),
        "f2" => (one_based, SequenceTerminator::Normal('Q')),
        "f3" => (one_based, SequenceTerminator::Normal('R')),
        "f4" => (one_based, SequenceTerminator::Normal('S')),
        "f5" => (Cow::Borrowed("15"), SequenceTerminator::Normal('~')),
        "f6" => (Cow::Borrowed("17"), SequenceTerminator::Normal('~')),
        "f7" => (Cow::Borrowed("18"), SequenceTerminator::Normal('~')),
        "f8" => (Cow::Borrowed("19"), SequenceTerminator::Normal('~')),
        "f9" => (Cow::Borrowed("20"), SequenceTerminator::Normal('~')),
        "f10" => (Cow::Borrowed("21"), SequenceTerminator::Normal('~')),
        "f11" => (Cow::Borrowed("23"), SequenceTerminator::Normal('~')),
        "f12" => (Cow::Borrowed("24"), SequenceTerminator::Normal('~')),
        _ => return None,
    };

    Some(SequenceBase::new(base, terminator))
}

fn try_build_kitty_control_char_or_modifier(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
    modifiers: &mut SequenceModifiers,
) -> Option<SequenceBase> {
    if !input_modes.kitty_report_all_keys_as_escape_codes && !input_modes.kitty_sequence_enabled() {
        return None;
    }

    let base = match event.keystroke.key.as_str() {
        "tab" => "9",
        "enter" => "13",
        "escape" => "27",
        "space" => "32",
        "backspace" => "127",
        "shift" if input_modes.kitty_report_all_keys_as_escape_codes => "57447",
        "control" if input_modes.kitty_report_all_keys_as_escape_codes => "57448",
        "alt" if input_modes.kitty_report_all_keys_as_escape_codes => "57449",
        "super" | "meta" if input_modes.kitty_report_all_keys_as_escape_codes => "57450",
        "capslock" if input_modes.kitty_report_all_keys_as_escape_codes => "57358",
        "numlock" if input_modes.kitty_report_all_keys_as_escape_codes => "57360",
        _ if input_modes.kitty_report_all_keys_as_escape_codes => "",
        _ => return None,
    };

    match event.keystroke.key.as_str() {
        "shift" => modifiers.set(
            SequenceModifier::Shift,
            event.phase != TerminalKeyPhase::Release,
        ),
        "control" => modifiers.set(
            SequenceModifier::Control,
            event.phase != TerminalKeyPhase::Release,
        ),
        "alt" => modifiers.set(
            SequenceModifier::Alt,
            event.phase != TerminalKeyPhase::Release,
        ),
        "super" | "meta" => modifiers.set(
            SequenceModifier::Super,
            event.phase != TerminalKeyPhase::Release,
        ),
        _ => {}
    }

    if base.is_empty() {
        None
    } else {
        Some(SequenceBase::new(base, SequenceTerminator::Kitty))
    }
}

fn associated_text<'a>(
    event: TerminalKeyEvent<'a>,
    input_modes: TerminalInputModes,
) -> Option<&'a str> {
    if !input_modes.kitty_report_associated_text || event.phase == TerminalKeyPhase::Release {
        return None;
    }

    let text = event.keystroke.key_char.as_deref()?;
    if text.is_empty() || is_control_character(text) {
        None
    } else {
        Some(text)
    }
}

fn base_text_key_char(keystroke: &gpui::Keystroke) -> Option<char> {
    let actual_char = keystroke.key_char.as_deref()?.chars().next()?;
    if keystroke.key.chars().count() == 1 {
        let key_char = keystroke.key.chars().next()?;
        if key_char.is_ascii_alphabetic() {
            Some(key_char.to_ascii_lowercase())
        } else {
            Some(key_char)
        }
    } else if keystroke.modifiers.shift {
        Some(actual_char.to_lowercase().next().unwrap_or(actual_char))
    } else {
        Some(actual_char)
    }
}

fn is_named_key_without_text(keystroke: &gpui::Keystroke) -> bool {
    keystroke.key_char.is_none()
        && matches!(
            keystroke.key.as_str(),
            "space"
                | "enter"
                | "tab"
                | "backspace"
                | "escape"
                | "up"
                | "down"
                | "left"
                | "right"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "insert"
                | "delete"
                | "f1"
                | "f2"
                | "f3"
                | "f4"
                | "f5"
                | "f6"
                | "f7"
                | "f8"
                | "f9"
                | "f10"
                | "f11"
                | "f12"
                | "f13"
                | "f14"
                | "f15"
                | "f16"
                | "f17"
                | "f18"
                | "f19"
                | "f20"
                | "f21"
                | "f22"
                | "f23"
                | "f24"
                | "f25"
                | "f26"
                | "f27"
                | "f28"
                | "f29"
                | "f30"
                | "f31"
                | "f32"
                | "f33"
                | "f34"
                | "f35"
                | "scrolllock"
                | "printscreen"
                | "pause"
                | "contextmenu"
                | "shift"
                | "control"
                | "alt"
                | "super"
                | "meta"
                | "capslock"
                | "numlock"
        )
}

fn is_best_effort_keypad_key(key: &str) -> bool {
    matches!(
        key,
        "numpad0"
            | "numpad1"
            | "numpad2"
            | "numpad3"
            | "numpad4"
            | "numpad5"
            | "numpad6"
            | "numpad7"
            | "numpad8"
            | "numpad9"
            | "numpaddecimal"
            | "numpaddivide"
            | "numpadmultiply"
            | "numpadsubtract"
            | "numpadadd"
            | "numpadenter"
            | "numpadequal"
            | "kp0"
            | "kp1"
            | "kp2"
            | "kp3"
            | "kp4"
            | "kp5"
            | "kp6"
            | "kp7"
            | "kp8"
            | "kp9"
            | "kp_0"
            | "kp_1"
            | "kp_2"
            | "kp_3"
            | "kp_4"
            | "kp_5"
            | "kp_6"
            | "kp_7"
            | "kp_8"
            | "kp_9"
            | "kpdecimal"
            | "kp_decimal"
            | "kpdivide"
            | "kp_divide"
            | "kpmultiply"
            | "kp_multiply"
            | "kpsubtract"
            | "kp_subtract"
            | "kpadd"
            | "kp_add"
            | "kpenter"
            | "kp_enter"
            | "kpequal"
            | "kp_equal"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SequenceModifier {
    Shift,
    Alt,
    Control,
    Super,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SequenceModifiers(u8);

impl SequenceModifiers {
    fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn set(&mut self, modifier: SequenceModifier, enabled: bool) {
        let bit = match modifier {
            SequenceModifier::Shift => 0b0000_0001,
            SequenceModifier::Alt => 0b0000_0010,
            SequenceModifier::Control => 0b0000_0100,
            SequenceModifier::Super => 0b0000_1000,
        };

        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    fn encode_esc_sequence(self) -> u8 {
        self.0 + 1
    }
}

fn sequence_modifiers(modifiers: gpui::Modifiers) -> SequenceModifiers {
    let mut sequence_modifiers = SequenceModifiers::default();
    sequence_modifiers.set(SequenceModifier::Shift, modifiers.shift);
    sequence_modifiers.set(SequenceModifier::Alt, modifiers.alt);
    sequence_modifiers.set(SequenceModifier::Control, modifiers.control);
    sequence_modifiers.set(SequenceModifier::Super, modifiers.platform);
    sequence_modifiers
}

struct SequenceBase {
    payload: String,
    terminator: SequenceTerminator,
}

impl SequenceBase {
    fn new(payload: impl Into<String>, terminator: SequenceTerminator) -> Self {
        Self {
            payload: payload.into(),
            terminator,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SequenceTerminator {
    Normal(char),
    Kitty,
}

impl SequenceTerminator {
    fn encode_esc_sequence(self) -> char {
        match self {
            SequenceTerminator::Normal(final_char) => final_char,
            SequenceTerminator::Kitty => 'u',
        }
    }
}

fn is_control_character(text: &str) -> bool {
    let codepoint = text.bytes().next().unwrap_or_default();
    text.len() == 1 && (codepoint < 0x20 || (0x7f..=0x9f).contains(&codepoint))
}

#[derive(Clone, Copy)]
struct LegacyKeyBinding {
    key: &'static str,
    shift: bool,
    alt: bool,
    control: bool,
    platform: bool,
    require_app_cursor: bool,
    prohibit_disambiguate_escape_codes: bool,
    prohibit_report_all_keys_as_escape_codes: bool,
    bytes: &'static [u8],
}

impl LegacyKeyBinding {
    fn matches(self, event: TerminalKeyEvent<'_>, input_modes: TerminalInputModes) -> bool {
        if event.phase == TerminalKeyPhase::Release {
            return false;
        }

        let modifiers = event.keystroke.modifiers;
        event.keystroke.key.as_str() == self.key
            && modifiers.shift == self.shift
            && modifiers.alt == self.alt
            && modifiers.control == self.control
            && modifiers.platform == self.platform
            && (!self.require_app_cursor || input_modes.app_cursor)
            && (!self.prohibit_disambiguate_escape_codes
                || !input_modes.kitty_disambiguate_escape_codes)
            && (!self.prohibit_report_all_keys_as_escape_codes
                || !input_modes.kitty_report_all_keys_as_escape_codes)
    }
}

const LEGACY_KEY_BINDINGS: &[LegacyKeyBinding] = &[
    LegacyKeyBinding {
        key: "home",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOH",
    },
    LegacyKeyBinding {
        key: "end",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOF",
    },
    LegacyKeyBinding {
        key: "up",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOA",
    },
    LegacyKeyBinding {
        key: "down",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOB",
    },
    LegacyKeyBinding {
        key: "right",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOC",
    },
    LegacyKeyBinding {
        key: "left",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: true,
        prohibit_disambiguate_escape_codes: false,
        prohibit_report_all_keys_as_escape_codes: false,
        bytes: b"\x1bOD",
    },
    LegacyKeyBinding {
        key: "numpadenter",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: false,
        prohibit_disambiguate_escape_codes: true,
        prohibit_report_all_keys_as_escape_codes: true,
        bytes: b"\n",
    },
    LegacyKeyBinding {
        key: "kpenter",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: false,
        prohibit_disambiguate_escape_codes: true,
        prohibit_report_all_keys_as_escape_codes: true,
        bytes: b"\n",
    },
    LegacyKeyBinding {
        key: "kp_enter",
        shift: false,
        alt: false,
        control: false,
        platform: false,
        require_app_cursor: false,
        prohibit_disambiguate_escape_codes: true,
        prohibit_report_all_keys_as_escape_codes: true,
        bytes: b"\n",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn key(
        key: &str,
        key_char: Option<&str>,
        shift: bool,
        alt: bool,
        control: bool,
        platform: bool,
    ) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: gpui::Modifiers {
                shift,
                control,
                alt,
                platform,
                function: false,
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    #[test]
    fn app_cursor_changes_arrow_sequence() {
        let binding = key("up", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                app_cursor: true,
                ..TerminalInputModes::default()
            },
        )
        .unwrap();
        assert_eq!(bytes, b"\x1bOA");
    }

    #[test]
    fn numpad_enter_defaults_to_newline() {
        let binding = key("numpadenter", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(event, TerminalInputModes::default()).unwrap();
        assert_eq!(bytes, b"\n");
    }

    #[test]
    fn return_key_defaults_to_carriage_return() {
        let binding = key("return", Some("\r"), false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(event, TerminalInputModes::default()).unwrap();
        assert_eq!(bytes, b"\r");
    }

    #[test]
    fn app_keypad_numpad_enter_overrides_legacy_newline() {
        let binding = key("numpadenter", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                app_keypad: true,
                ..TerminalInputModes::default()
            },
        )
        .unwrap();
        assert_eq!(bytes, b"\x1bOM");
    }

    #[test]
    fn modified_named_key_uses_xterm_parameter() {
        let binding = key("left", None, false, true, true, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(event, TerminalInputModes::default()).unwrap();
        assert_eq!(bytes, b"\x1b[1;7D");
    }

    #[test]
    fn kitty_textual_sequence_reports_alternate_key() {
        let binding = key("1", Some("!"), true, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Press);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                kitty_keyboard_protocol: true,
                kitty_disambiguate_escape_codes: true,
                kitty_report_alternate_keys: true,
                ..TerminalInputModes::default()
            },
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[49:33u");
    }

    #[test]
    fn kitty_repeat_sequence_reports_event_type() {
        let binding = key("up", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Repeat);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                kitty_keyboard_protocol: true,
                kitty_report_event_types: true,
                ..TerminalInputModes::default()
            },
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[1;1:2A");
    }

    #[test]
    fn kitty_release_sequence_reports_event_type() {
        let binding = key("up", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Release);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                kitty_keyboard_protocol: true,
                kitty_report_event_types: true,
                ..TerminalInputModes::default()
            },
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[1;1:3A");
    }

    #[test]
    fn release_without_report_event_types_is_ignored() {
        let binding = key("up", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Release);
        let bytes = encode_terminal_input(event, TerminalInputModes::default());
        assert_eq!(bytes, None);
    }

    #[test]
    fn kitty_release_without_event_types_is_ignored() {
        let binding = key("escape", None, false, false, false, false);
        let event = TerminalKeyEvent::new(&binding, TerminalKeyPhase::Release);
        let bytes = encode_terminal_input(
            event,
            TerminalInputModes {
                kitty_keyboard_protocol: true,
                kitty_disambiguate_escape_codes: true,
                ..TerminalInputModes::default()
            },
        );
        assert_eq!(bytes, None);
    }
}

#[cfg(test)]
mod paste_tests {
    use super::sanitize_paste;

    #[test]
    fn sanitize_paste_normalizes_newlines_and_controls() {
        assert_eq!(sanitize_paste("a\n\u{0000}b\u{007f}c\t", false), b"a\rbc\t");
    }

    #[test]
    fn sanitize_paste_wraps_bracketed_payloads() {
        assert_eq!(sanitize_paste("hello", true), b"[200~hello[201~");
    }
}
