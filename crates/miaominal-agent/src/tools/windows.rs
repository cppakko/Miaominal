use base64::Engine as _;

pub(crate) fn powershell_encoded_payload(script: &str) -> String {
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn powershell_encoded_command(script: &str) -> String {
    format!(
        "powershell.exe -NoProfile -EncodedCommand {}",
        powershell_encoded_payload(script)
    )
}
