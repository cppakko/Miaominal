use base64::Engine as _;
use flate2::Compression;
use flate2::write::GzEncoder;
use miaominal_core::profile::ShellType;
use std::io::Write as _;

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

pub(crate) fn powershell_command_for_shell(shell_type: ShellType, script: &str) -> String {
    match shell_type {
        ShellType::PowerShell => powershell_encoded_command(script),
        ShellType::Cmd => powershell_compressed_command_for_cmd(script),
        _ => unreachable!("PowerShell wrapper requested for non-Windows shell"),
    }
}

pub(crate) fn powershell_sensitive_path_function() -> &'static str {
    r#"function Test-MiaominalSensitivePath([string]$Path) {
  $normalized=$Path.Replace('\','/').ToLowerInvariant();
  $parts=@($normalized.Split('/',[StringSplitOptions]::RemoveEmptyEntries));
  $base=if($parts.Count -gt 0){$parts[$parts.Count-1]}else{''};
  if($parts -contains '.ssh'){return $true};
  if(@('id_rsa','id_dsa','id_ecdsa','id_ed25519') -contains $base){return $true};
  if($base.EndsWith('.env') -or $base.Contains('.env.') -or $base.EndsWith('.pem') -or $base.EndsWith('.key') -or $base.EndsWith('.p12') -or $base.EndsWith('.pfx') -or $base.EndsWith('.rdp') -or $base.EndsWith('.kdbx')){return $true};
  $windows=$normalized.TrimStart('/');
  if($windows.StartsWith('c:/windows/system32/config/') -or $normalized.Contains('ntds.dit') -or $normalized.Contains('appdata/roaming/mozilla/firefox/profiles') -or $normalized.Contains('appdata/local/google/chrome/user data/default')){return $true};
  return $false
}"#
}

/// Encode a large PowerShell script behind a small gzip bootstrap.
///
/// CMD limits command lines to roughly 8 KiB. Plain UTF-16 `EncodedCommand`
/// expands the job-management scripts beyond that limit, so compress the real
/// script and only encode the compact decompressor as UTF-16.
pub(crate) fn powershell_compressed_command(script: &str) -> String {
    let payload = powershell_compressed_payload(script);
    let bootstrap = format!(
        concat!(
            "$b=[Convert]::FromBase64String('{payload}');",
            "$m=New-Object IO.MemoryStream(,$b);",
            "$g=New-Object IO.Compression.GzipStream($m,[IO.Compression.CompressionMode]::Decompress);",
            "$r=New-Object IO.StreamReader($g,[Text.Encoding]::UTF8);",
            "& ([ScriptBlock]::Create($r.ReadToEnd()))"
        ),
        payload = payload,
    );
    powershell_encoded_command(&bootstrap)
}

/// Encode a large PowerShell script for a CMD exec channel without encoding
/// the compressed payload twice. The base64 gzip data is safe in an unquoted
/// `set NAME=value` assignment; the short encoded bootstrap removes it before
/// real script starts, so detached children do not inherit the staging value.
pub(crate) fn powershell_compressed_command_for_cmd(script: &str) -> String {
    const ENV_NAME: &str = "MIAOMINAL_AGENT_PS_GZIP";
    let payload = powershell_compressed_payload(script);
    let bootstrap = format!(
        concat!(
            "$p=$env:{env_name};",
            "Remove-Item Env:{env_name} -ErrorAction SilentlyContinue;",
            "$b=[Convert]::FromBase64String($p);",
            "$m=New-Object IO.MemoryStream(,$b);",
            "$g=New-Object IO.Compression.GzipStream($m,[IO.Compression.CompressionMode]::Decompress);",
            "$r=New-Object IO.StreamReader($g,[Text.Encoding]::UTF8);",
            "& ([ScriptBlock]::Create($r.ReadToEnd()))"
        ),
        env_name = ENV_NAME,
    );
    format!(
        "set {ENV_NAME}={payload}& {}",
        powershell_encoded_command(&bootstrap)
    )
}

pub(crate) fn powershell_compressed_payload(script: &str) -> String {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(script.as_bytes())
        .expect("writing gzip data to memory cannot fail");
    let compressed = encoder
        .finish()
        .expect("finishing gzip data in memory cannot fail");
    base64::engine::general_purpose::STANDARD.encode(compressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_command_stays_below_cmd_limit_for_repetitive_scripts() {
        let script = "Write-Output 'hello';".repeat(2_000);
        let command = powershell_compressed_command(&script);
        assert!(command.len() < 8_191, "command was {} bytes", command.len());
    }

    #[test]
    fn cmd_compressed_command_stages_payload_only_once() {
        let script = "Write-Output 'hello';".repeat(2_000);
        let command = powershell_compressed_command_for_cmd(&script);

        assert!(command.starts_with("set MIAOMINAL_AGENT_PS_GZIP="));
        assert!(command.contains("& powershell.exe -NoProfile -EncodedCommand "));
        assert!(command.len() < 8_191, "command was {} bytes", command.len());
    }
}
