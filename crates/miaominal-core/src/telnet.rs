//! Platform independent Telnet (RFC 854) protocol codec.
//!
//! The codec is transport agnostic: it turns raw bytes received from the
//! network into terminal output plus negotiation replies, and turns terminal
//! input into network bytes. Option negotiation follows the loop-free "Q
//! method" (RFC 1143) for the options a terminal client supports:
//!
//! - local (we send): `BINARY`, `SUPPRESS-GO-AHEAD`, `TERMINAL-TYPE`, `NAWS`
//! - remote (server sends): `BINARY`, `ECHO`, `SUPPRESS-GO-AHEAD`
//!
//! Every other option is refused.

pub const DEFAULT_TELNET_PORT: u16 = 23;
pub const DEFAULT_TELNET_TERMINAL_TYPE: &str = "XTERM-256COLOR";

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const OPTION_BINARY: u8 = 0;
pub const OPTION_ECHO: u8 = 1;
pub const OPTION_SUPPRESS_GO_AHEAD: u8 = 3;
pub const OPTION_TERMINAL_TYPE: u8 = 24;
pub const OPTION_NAWS: u8 = 31;

const TERMINAL_TYPE_IS: u8 = 0;
const TERMINAL_TYPE_SEND: u8 = 1;
/// Subnegotiations longer than this are discarded instead of buffered.
const MAX_SUBNEGOTIATION_LEN: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OptionState {
    #[default]
    No,
    WantYes,
    Yes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ParseState {
    #[default]
    Data,
    /// A carriage return was emitted; a following NUL is dropped.
    CarriageReturn,
    Iac,
    Negotiation(u8),
    Subnegotiation,
    SubnegotiationIac,
}

/// Bytes produced by [`TelnetCodec::receive`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TelnetReceived {
    /// Terminal output with Telnet commands removed.
    pub data: Vec<u8>,
    /// Negotiation replies that must be written back to the server.
    pub reply: Vec<u8>,
}

#[derive(Debug)]
pub struct TelnetCodec {
    terminal_type: String,
    columns: u16,
    lines: u16,
    local: [OptionState; 256],
    remote: [OptionState; 256],
    parse: ParseState,
    subnegotiation: Vec<u8>,
    subnegotiation_overflow: bool,
}

impl TelnetCodec {
    pub fn new(terminal_type: impl Into<String>, columns: usize, lines: usize) -> Self {
        Self {
            terminal_type: terminal_type.into(),
            columns: clamp_dimension(columns),
            lines: clamp_dimension(lines),
            local: [OptionState::No; 256],
            remote: [OptionState::No; 256],
            parse: ParseState::Data,
            subnegotiation: Vec::new(),
            subnegotiation_overflow: false,
        }
    }

    /// Opening offers sent right after the transport connects.
    pub fn initial_negotiation(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for option in [OPTION_TERMINAL_TYPE, OPTION_NAWS] {
            self.request_local(option, &mut out);
        }
        for option in [OPTION_SUPPRESS_GO_AHEAD, OPTION_ECHO] {
            self.request_remote(option, &mut out);
        }
        out
    }

    /// Whether the server agreed to echo input (character-at-a-time mode).
    pub fn remote_echo(&self) -> bool {
        self.remote[OPTION_ECHO as usize] == OptionState::Yes
    }

    pub fn receive(&mut self, input: &[u8]) -> TelnetReceived {
        let mut received = TelnetReceived::default();
        for &byte in input {
            self.receive_byte(byte, &mut received);
        }
        received
    }

    /// Encodes terminal input for the network: `IAC` is doubled and, unless
    /// binary transmission was negotiated, a bare carriage return becomes
    /// `CR NUL` as required by the Network Virtual Terminal.
    pub fn encode_input(&self, input: &[u8]) -> Vec<u8> {
        let binary = self.local[OPTION_BINARY as usize] == OptionState::Yes;
        let mut out = Vec::with_capacity(input.len() + 4);
        for (index, &byte) in input.iter().enumerate() {
            match byte {
                IAC => out.extend_from_slice(&[IAC, IAC]),
                b'\r' if !binary && input.get(index + 1) != Some(&b'\n') => {
                    out.extend_from_slice(b"\r\0")
                }
                byte => out.push(byte),
            }
        }
        out
    }

    /// Records a new window size and returns the `NAWS` update to send, if the
    /// server accepted window size reporting.
    pub fn resize(&mut self, columns: usize, lines: usize) -> Vec<u8> {
        let columns = clamp_dimension(columns);
        let lines = clamp_dimension(lines);
        if (columns, lines) == (self.columns, self.lines) {
            return Vec::new();
        }
        self.columns = columns;
        self.lines = lines;
        let mut out = Vec::new();
        if self.local[OPTION_NAWS as usize] == OptionState::Yes {
            self.write_window_size(&mut out);
        }
        out
    }

    fn receive_byte(&mut self, byte: u8, received: &mut TelnetReceived) {
        match self.parse {
            ParseState::Data | ParseState::CarriageReturn => {
                let after_carriage_return = self.parse == ParseState::CarriageReturn;
                self.parse = ParseState::Data;
                match byte {
                    IAC => self.parse = ParseState::Iac,
                    0 if after_carriage_return && !self.remote_binary() => {}
                    b'\r' => {
                        received.data.push(byte);
                        self.parse = ParseState::CarriageReturn;
                    }
                    byte => received.data.push(byte),
                }
            }
            ParseState::Iac => {
                self.parse = ParseState::Data;
                match byte {
                    IAC => received.data.push(IAC),
                    DO | DONT | WILL | WONT => self.parse = ParseState::Negotiation(byte),
                    SB => {
                        self.subnegotiation.clear();
                        self.subnegotiation_overflow = false;
                        self.parse = ParseState::Subnegotiation;
                    }
                    // NOP, GA, DM, BRK and the other commands carry no data.
                    _ => {}
                }
            }
            ParseState::Negotiation(verb) => {
                self.parse = ParseState::Data;
                match verb {
                    DO => self.on_do(byte, &mut received.reply),
                    DONT => self.on_dont(byte, &mut received.reply),
                    WILL => self.on_will(byte, &mut received.reply),
                    _ => self.on_wont(byte, &mut received.reply),
                }
            }
            ParseState::Subnegotiation => match byte {
                IAC => self.parse = ParseState::SubnegotiationIac,
                byte => self.push_subnegotiation(byte),
            },
            ParseState::SubnegotiationIac => match byte {
                SE => {
                    self.parse = ParseState::Data;
                    if !self.subnegotiation_overflow {
                        self.on_subnegotiation(&mut received.reply);
                    }
                }
                IAC => {
                    self.push_subnegotiation(IAC);
                    self.parse = ParseState::Subnegotiation;
                }
                // Malformed: treat as the end of the subnegotiation.
                _ => self.parse = ParseState::Data,
            },
        }
    }

    fn push_subnegotiation(&mut self, byte: u8) {
        if self.subnegotiation.len() >= MAX_SUBNEGOTIATION_LEN {
            self.subnegotiation_overflow = true;
        } else {
            self.subnegotiation.push(byte);
        }
    }

    fn remote_binary(&self) -> bool {
        self.remote[OPTION_BINARY as usize] == OptionState::Yes
    }

    fn supports_local(option: u8) -> bool {
        matches!(
            option,
            OPTION_BINARY | OPTION_SUPPRESS_GO_AHEAD | OPTION_TERMINAL_TYPE | OPTION_NAWS
        )
    }

    fn supports_remote(option: u8) -> bool {
        matches!(
            option,
            OPTION_BINARY | OPTION_ECHO | OPTION_SUPPRESS_GO_AHEAD
        )
    }

    fn request_local(&mut self, option: u8, out: &mut Vec<u8>) {
        if self.local[option as usize] == OptionState::No {
            self.local[option as usize] = OptionState::WantYes;
            out.extend_from_slice(&[IAC, WILL, option]);
        }
    }

    fn request_remote(&mut self, option: u8, out: &mut Vec<u8>) {
        if self.remote[option as usize] == OptionState::No {
            self.remote[option as usize] = OptionState::WantYes;
            out.extend_from_slice(&[IAC, DO, option]);
        }
    }

    fn on_do(&mut self, option: u8, out: &mut Vec<u8>) {
        let index = option as usize;
        match self.local[index] {
            OptionState::No if Self::supports_local(option) => {
                self.local[index] = OptionState::Yes;
                out.extend_from_slice(&[IAC, WILL, option]);
                self.local_enabled(option, out);
            }
            OptionState::No => out.extend_from_slice(&[IAC, WONT, option]),
            OptionState::WantYes => {
                self.local[index] = OptionState::Yes;
                self.local_enabled(option, out);
            }
            // A repeated DO for an option that is already on needs no answer.
            OptionState::Yes => {}
        }
    }

    fn on_dont(&mut self, option: u8, out: &mut Vec<u8>) {
        let index = option as usize;
        match self.local[index] {
            OptionState::Yes => {
                self.local[index] = OptionState::No;
                out.extend_from_slice(&[IAC, WONT, option]);
            }
            OptionState::WantYes => self.local[index] = OptionState::No,
            OptionState::No => {}
        }
    }

    fn on_will(&mut self, option: u8, out: &mut Vec<u8>) {
        let index = option as usize;
        match self.remote[index] {
            OptionState::No if Self::supports_remote(option) => {
                self.remote[index] = OptionState::Yes;
                out.extend_from_slice(&[IAC, DO, option]);
            }
            OptionState::No => out.extend_from_slice(&[IAC, DONT, option]),
            OptionState::WantYes => self.remote[index] = OptionState::Yes,
            OptionState::Yes => {}
        }
    }

    fn on_wont(&mut self, option: u8, out: &mut Vec<u8>) {
        let index = option as usize;
        match self.remote[index] {
            OptionState::Yes => {
                self.remote[index] = OptionState::No;
                out.extend_from_slice(&[IAC, DONT, option]);
            }
            OptionState::WantYes => self.remote[index] = OptionState::No,
            OptionState::No => {}
        }
    }

    fn local_enabled(&mut self, option: u8, out: &mut Vec<u8>) {
        if option == OPTION_NAWS {
            self.write_window_size(out);
        }
    }

    fn on_subnegotiation(&mut self, out: &mut Vec<u8>) {
        if self.subnegotiation.as_slice() == [OPTION_TERMINAL_TYPE, TERMINAL_TYPE_SEND]
            && self.local[OPTION_TERMINAL_TYPE as usize] == OptionState::Yes
        {
            out.extend_from_slice(&[IAC, SB, OPTION_TERMINAL_TYPE, TERMINAL_TYPE_IS]);
            push_escaped(out, self.terminal_type.as_bytes());
            out.extend_from_slice(&[IAC, SE]);
        }
    }

    fn write_window_size(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[IAC, SB, OPTION_NAWS]);
        let mut size = [0u8; 4];
        size[..2].copy_from_slice(&self.columns.to_be_bytes());
        size[2..].copy_from_slice(&self.lines.to_be_bytes());
        push_escaped(out, &size);
        out.extend_from_slice(&[IAC, SE]);
    }
}

fn push_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for &byte in bytes {
        if byte == IAC {
            out.push(IAC);
        }
        out.push(byte);
    }
}

fn clamp_dimension(value: usize) -> u16 {
    value.clamp(1, u16::MAX as usize) as u16
}

/// Detects login prompts in Telnet output so a profile's saved username and
/// password can be typed automatically, once each, and tells the session when
/// the login is over so a startup command is not typed into a login prompt.
#[derive(Debug)]
pub struct TelnetAutoLogin {
    username: Option<String>,
    password: Option<String>,
    has_credentials: bool,
    awaiting_manual_password: bool,
    logged_in: bool,
    tail: String,
    observed: usize,
}

/// Prompts are only watched for in the first part of the session.
const AUTO_LOGIN_OBSERVATION_LIMIT: usize = 64 * 1024;
const AUTO_LOGIN_TAIL_LIMIT: usize = 128;

impl TelnetAutoLogin {
    pub fn new(username: &str, password: &str) -> Self {
        let username = username.trim();
        let username = (!username.is_empty()).then(|| username.to_string());
        let password = (!password.is_empty()).then(|| password.to_string());
        Self {
            has_credentials: username.is_some() || password.is_some(),
            username,
            password,
            awaiting_manual_password: false,
            logged_in: false,
            tail: String::new(),
            observed: 0,
        }
    }

    /// Whether the login phase is over: a profile without credentials has no
    /// login phase, otherwise the password has been sent (automatically or by
    /// the user), or the prompt watch gave up.
    pub fn ready_for_startup(&self) -> bool {
        !self.has_credentials || self.logged_in || self.observed >= AUTO_LOGIN_OBSERVATION_LIMIT
    }

    /// Records input typed by the user; Enter at a password prompt that had no
    /// saved password completes the login.
    pub fn note_user_input(&mut self, input: &[u8]) {
        if self.awaiting_manual_password && input.contains(&b'\r') {
            self.awaiting_manual_password = false;
            self.logged_in = true;
        }
    }

    /// Feeds terminal output and returns a line to type, if a prompt matched.
    /// The returned line ends with a carriage return.
    pub fn observe(&mut self, output: &[u8]) -> Option<String> {
        if self.ready_for_startup() {
            return None;
        }
        self.observed = self.observed.saturating_add(output.len());
        self.tail
            .push_str(&String::from_utf8_lossy(output).to_ascii_lowercase());
        if self.tail.len() > AUTO_LOGIN_TAIL_LIMIT {
            let mut cut = self.tail.len() - AUTO_LOGIN_TAIL_LIMIT;
            while !self.tail.is_char_boundary(cut) {
                cut += 1;
            }
            self.tail.drain(..cut);
        }
        let prompt = self.tail.trim_end();
        if prompt.ends_with("password:") {
            self.tail.clear();
            // A password prompt also ends the username phase.
            self.username = None;
            return match self.password.take() {
                Some(password) => {
                    self.logged_in = true;
                    Some(format!("{password}\r"))
                }
                None => {
                    self.awaiting_manual_password = true;
                    None
                }
            };
        }
        if prompt.ends_with("login:")
            || prompt.ends_with("username:")
            || prompt.ends_with("user name:")
        {
            let username = self.username.take()?;
            self.tail.clear();
            return Some(format!("{username}\r"));
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> TelnetCodec {
        TelnetCodec::new("XTERM-256COLOR", 80, 24)
    }

    #[test]
    fn plain_data_passes_through() {
        let mut codec = codec();
        let received = codec.receive(b"hello\r\nworld");
        assert_eq!(received.data, b"hello\r\nworld");
        assert!(received.reply.is_empty());
    }

    #[test]
    fn escaped_iac_and_cr_nul_are_decoded() {
        let mut codec = codec();
        let received = codec.receive(&[b'a', IAC, IAC, b'b', b'\r', 0, b'c']);
        assert_eq!(received.data, vec![b'a', IAC, b'b', b'\r', b'c']);
    }

    #[test]
    fn commands_split_across_reads_are_parsed() {
        let mut codec = codec();
        let first = codec.receive(&[b'x', IAC]);
        let second = codec.receive(&[WILL]);
        let third = codec.receive(&[OPTION_ECHO, b'y']);
        assert_eq!(first.data, b"x");
        assert!(second.data.is_empty());
        assert_eq!(third.data, b"y");
        assert_eq!(third.reply, vec![IAC, DO, OPTION_ECHO]);
        assert!(codec.remote_echo());
    }

    #[test]
    fn initial_offers_are_not_answered_twice() {
        let mut codec = codec();
        let offers = codec.initial_negotiation();
        assert_eq!(
            offers,
            vec![
                IAC,
                WILL,
                OPTION_TERMINAL_TYPE,
                IAC,
                WILL,
                OPTION_NAWS,
                IAC,
                DO,
                OPTION_SUPPRESS_GO_AHEAD,
                IAC,
                DO,
                OPTION_ECHO,
            ]
        );
        let received = codec.receive(&[IAC, WILL, OPTION_ECHO, IAC, DO, OPTION_TERMINAL_TYPE]);
        assert!(received.reply.is_empty());
        assert!(codec.remote_echo());
    }

    #[test]
    fn accepting_naws_reports_the_window_size() {
        let mut codec = codec();
        let received = codec.receive(&[IAC, DO, OPTION_NAWS]);
        assert_eq!(
            received.reply,
            vec![
                IAC,
                WILL,
                OPTION_NAWS,
                IAC,
                SB,
                OPTION_NAWS,
                0,
                80,
                0,
                24,
                IAC,
                SE
            ]
        );
        assert_eq!(
            codec.resize(255, 50),
            vec![IAC, SB, OPTION_NAWS, 0, IAC, IAC, 0, 50, IAC, SE]
        );
        assert!(codec.resize(255, 50).is_empty());
    }

    #[test]
    fn resize_is_silent_until_naws_is_enabled() {
        let mut codec = codec();
        assert!(codec.resize(100, 30).is_empty());
    }

    #[test]
    fn terminal_type_is_sent_on_request() {
        let mut codec = codec();
        codec.receive(&[IAC, DO, OPTION_TERMINAL_TYPE]);
        let received = codec.receive(&[IAC, SB, OPTION_TERMINAL_TYPE, TERMINAL_TYPE_SEND, IAC, SE]);
        let mut expected = vec![IAC, SB, OPTION_TERMINAL_TYPE, TERMINAL_TYPE_IS];
        expected.extend_from_slice(b"XTERM-256COLOR");
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(received.reply, expected);
    }

    #[test]
    fn unsupported_options_are_refused() {
        let mut codec = codec();
        let received = codec.receive(&[IAC, DO, 39, IAC, WILL, 34]);
        assert_eq!(received.reply, vec![IAC, WONT, 39, IAC, DONT, 34]);
    }

    #[test]
    fn disabling_an_enabled_option_is_acknowledged_once() {
        let mut codec = codec();
        codec.receive(&[IAC, WILL, OPTION_ECHO]);
        let received = codec.receive(&[IAC, WONT, OPTION_ECHO, IAC, WONT, OPTION_ECHO]);
        assert_eq!(received.reply, vec![IAC, DONT, OPTION_ECHO]);
        assert!(!codec.remote_echo());
    }

    #[test]
    fn input_escapes_iac_and_bare_carriage_returns() {
        let codec = codec();
        assert_eq!(codec.encode_input(b"ls\r"), b"ls\r\0");
        assert_eq!(codec.encode_input(b"a\r\nb"), b"a\r\nb");
        assert_eq!(codec.encode_input(&[IAC]), vec![IAC, IAC]);
    }

    #[test]
    fn binary_transmission_keeps_carriage_returns_raw() {
        let mut codec = codec();
        codec.receive(&[IAC, DO, OPTION_BINARY]);
        assert_eq!(codec.encode_input(b"ls\r"), b"ls\r");
    }

    #[test]
    fn oversized_subnegotiations_are_dropped() {
        let mut codec = codec();
        codec.receive(&[IAC, DO, OPTION_TERMINAL_TYPE]);
        let mut input = vec![IAC, SB, OPTION_TERMINAL_TYPE, TERMINAL_TYPE_SEND];
        input.extend(std::iter::repeat_n(b'x', MAX_SUBNEGOTIATION_LEN + 10));
        input.extend_from_slice(&[IAC, SE, b'z']);
        let received = codec.receive(&input);
        assert!(received.reply.is_empty());
        assert_eq!(received.data, b"z");
    }

    #[test]
    fn auto_login_answers_each_prompt_once() {
        let mut login = TelnetAutoLogin::new(" admin ", "hunter2");
        assert!(!login.ready_for_startup());
        assert_eq!(login.observe(b"Welcome\r\n"), None);
        assert_eq!(login.observe(b"router Login: "), Some("admin\r".into()));
        assert_eq!(login.observe(b"login: "), None);
        assert_eq!(login.observe(b"Pass"), None);
        assert_eq!(login.observe(b"word: "), Some("hunter2\r".into()));
        assert!(login.ready_for_startup());
    }

    #[test]
    fn auto_login_without_credentials_is_ready_immediately() {
        let mut login = TelnetAutoLogin::new("", "");
        assert!(login.ready_for_startup());
        assert_eq!(login.observe(b"login: "), None);
    }

    #[test]
    fn password_prompt_skips_a_pending_username() {
        let mut login = TelnetAutoLogin::new("admin", "secret");
        assert_eq!(login.observe(b"Password: "), Some("secret\r".into()));
        assert!(login.ready_for_startup());
    }

    #[test]
    fn manual_password_entry_completes_the_login() {
        let mut login = TelnetAutoLogin::new("admin", "");
        assert_eq!(login.observe(b"login: "), Some("admin\r".into()));
        assert_eq!(login.observe(b"Password: "), None);
        login.note_user_input(b"hun");
        assert!(!login.ready_for_startup());
        login.note_user_input(b"ter2\r");
        assert!(login.ready_for_startup());
    }

    #[test]
    fn last_login_banners_do_not_count_as_prompts() {
        let mut login = TelnetAutoLogin::new("admin", "");
        assert_eq!(
            login.observe(b"Last login: Mon Nov  2 13:10 2026\r\n"),
            None
        );
    }
}
