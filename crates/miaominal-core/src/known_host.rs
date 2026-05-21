#[derive(Clone, Debug)]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Debug)]
pub enum HostKeyCheck {
    Match,
    Unknown,
    Mismatch { line: usize },
}
