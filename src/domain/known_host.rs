use russh::keys::{HashAlg, PublicKey};

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

pub fn fingerprint_of(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

pub fn algorithm_of(key: &PublicKey) -> String {
    key.algorithm().to_string()
}
