use crate::backend::BackendRoute;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteCapabilities {
    pub sftp: bool,
    pub exec: bool,
    pub pty: bool,
    pub rg: bool,
    pub git: bool,
    pub patch: bool,
    pub python3: bool,
    pub grep: bool,
    pub find: bool,
    pub sed: bool,
    pub powershell: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityProbeResult {
    pub home: String,
    pub cwd: String,
    pub user: String,
    pub platform: String,
    pub arch: String,
    pub capabilities: RemoteCapabilities,
    pub route: BackendRoute,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityProbe;

impl CapabilityProbe {
    pub fn posix_command() -> &'static str {
        "printf 'home=%s\\npwd=%s\\nuser=%s\\n' \"$HOME\" \"$PWD\" \"$(id -un 2>/dev/null || whoami)\"; \
         printf 'platform=%s\\narch=%s\\n' \"$(uname -s 2>/dev/null || printf unknown)\" \"$(uname -m 2>/dev/null || printf unknown)\"; \
         for cmd in rg git patch python3 grep find sed pwsh powershell; do command -v \"$cmd\" >/dev/null 2>&1 && printf 'cap_%s=1\\n' \"$cmd\" || printf 'cap_%s=0\\n' \"$cmd\"; done"
    }

    pub fn parse_posix(output: &str, route: BackendRoute) -> CapabilityProbeResult {
        CapabilityProbeResult {
            home: value_for(output, "home").unwrap_or_default(),
            cwd: value_for(output, "pwd").unwrap_or_default(),
            user: value_for(output, "user").unwrap_or_default(),
            platform: value_for(output, "platform").unwrap_or_else(|| "unknown".into()),
            arch: value_for(output, "arch").unwrap_or_else(|| "unknown".into()),
            capabilities: RemoteCapabilities {
                sftp: false,
                exec: true,
                pty: false,
                rg: has_cap(output, "rg"),
                git: has_cap(output, "git"),
                patch: has_cap(output, "patch"),
                python3: has_cap(output, "python3"),
                grep: has_cap(output, "grep"),
                find: has_cap(output, "find"),
                sed: has_cap(output, "sed"),
                powershell: has_cap(output, "powershell") || has_cap(output, "pwsh"),
            },
            route,
        }
    }
}

fn value_for(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    output
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::to_string))
}

fn has_cap(output: &str, name: &str) -> bool {
    output.lines().any(|line| line == format!("cap_{name}=1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_posix_probe_result() {
        let probe = CapabilityProbe::parse_posix(
            "home=/home/deploy\npwd=/srv/app\nuser=deploy\nplatform=Linux\narch=x86_64\ncap_rg=1\ncap_git=0\ncap_python3=1",
            BackendRoute::SshExec,
        );

        assert_eq!(probe.home, "/home/deploy");
        assert_eq!(probe.cwd, "/srv/app");
        assert_eq!(probe.user, "deploy");
        assert_eq!(probe.platform, "Linux");
        assert!(probe.capabilities.rg);
        assert!(!probe.capabilities.git);
        assert!(probe.capabilities.python3);
    }
}
