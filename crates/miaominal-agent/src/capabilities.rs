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
    pub shell: String,
    pub capabilities: RemoteCapabilities,
    pub route: BackendRoute,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityProbe;

impl CapabilityProbe {
    pub fn posix_command() -> &'static str {
        "printf 'home=%s\\npwd=%s\\nuser=%s\\n' \"$HOME\" \"$PWD\" \"$(id -un 2>/dev/null || whoami)\"; \
         printf 'platform=%s\\narch=%s\\nshell=%s\\n' \"$(uname -s 2>/dev/null || printf unknown)\" \"$(uname -m 2>/dev/null || printf unknown)\" \"${SHELL##*/}\"; \
         for cmd in rg git patch python3 grep find sed pwsh powershell; do command -v \"$cmd\" >/dev/null 2>&1 && printf 'cap_%s=1\\n' \"$cmd\" || printf 'cap_%s=0\\n' \"$cmd\"; done"
    }

    pub fn parse_posix(output: &str, route: BackendRoute) -> CapabilityProbeResult {
        CapabilityProbeResult {
            home: value_for(output, "home").unwrap_or_default(),
            cwd: value_for(output, "pwd").unwrap_or_default(),
            user: value_for(output, "user").unwrap_or_default(),
            platform: value_for(output, "platform").unwrap_or_else(|| "unknown".into()),
            arch: value_for(output, "arch").unwrap_or_else(|| "unknown".into()),
            shell: value_for(output, "shell").unwrap_or_else(|| "posix-sh".into()),
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

    pub fn powershell_command() -> &'static str {
        // Single-quoted strings inside the PS script avoid double-quote
        // conflicts with the outer `powershell.exe -NoProfile -Command "..."` wrapper.
        "powershell.exe -NoProfile -Command \"\
         Write-Output 'home=' + $env:USERPROFILE; \
         Write-Output 'pwd=' + (Get-Location).Path; \
         Write-Output 'user=' + $env:USERNAME; \
         Write-Output 'platform=Windows'; \
         if ([System.Environment]::Is64BitOperatingSystem) { \
             Write-Output 'arch=x86_64' \
         } else { \
             Write-Output 'arch=x86' \
         }; \
         Write-Output 'shell=powershell'; \
         @('rg','git','patch','python3','grep','find','sed','pwsh','powershell') | ForEach-Object { \
             if (Get-Command $_ -ErrorAction SilentlyContinue) { \
                 Write-Output ('cap_' + $_ + '=1') \
             } else { \
                 Write-Output ('cap_' + $_ + '=0') \
             } \
         }\""
    }

    pub fn cmd_command() -> &'static str {
        "@echo off & \
         echo home=%USERPROFILE% & \
         echo pwd=%CD% & \
         echo user=%USERNAME% & \
         echo platform=Windows & \
         if defined PROCESSOR_ARCHITEW6432 (echo arch=x86_64) else if /I \"%PROCESSOR_ARCHITECTURE%\"==\"AMD64\" (echo arch=x86_64) else if /I \"%PROCESSOR_ARCHITECTURE%\"==\"ARM64\" (echo arch=aarch64) else (echo arch=x86) & \
         echo shell=cmd & \
         where rg >nul 2>&1 && echo cap_rg=1 || echo cap_rg=0 & \
         where git >nul 2>&1 && echo cap_git=1 || echo cap_git=0 & \
         where patch >nul 2>&1 && echo cap_patch=1 || echo cap_patch=0 & \
         where python3 >nul 2>&1 && echo cap_python3=1 || echo cap_python3=0 & \
         where grep >nul 2>&1 && echo cap_grep=1 || echo cap_grep=0 & \
         where find >nul 2>&1 && echo cap_find=1 || echo cap_find=0 & \
         where sed >nul 2>&1 && echo cap_sed=1 || echo cap_sed=0 & \
         where pwsh >nul 2>&1 && echo cap_pwsh=1 || echo cap_pwsh=0 & \
         where powershell >nul 2>&1 && echo cap_powershell=1 || echo cap_powershell=0"
    }

    pub fn parse_powershell(output: &str, route: BackendRoute) -> CapabilityProbeResult {
        CapabilityProbeResult {
            home: value_for(output, "home").unwrap_or_default(),
            cwd: value_for(output, "pwd").unwrap_or_default(),
            user: value_for(output, "user").unwrap_or_default(),
            platform: value_for(output, "platform").unwrap_or_else(|| "Windows".into()),
            arch: value_for(output, "arch").unwrap_or_else(|| "unknown".into()),
            shell: value_for(output, "shell").unwrap_or_else(|| "powershell".into()),
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
                powershell: true,
            },
            route,
        }
    }

    pub fn parse_cmd(output: &str, route: BackendRoute) -> CapabilityProbeResult {
        CapabilityProbeResult {
            home: value_for(output, "home").unwrap_or_default(),
            cwd: value_for(output, "pwd").unwrap_or_default(),
            user: value_for(output, "user").unwrap_or_default(),
            platform: value_for(output, "platform").unwrap_or_else(|| "Windows".into()),
            arch: value_for(output, "arch").unwrap_or_else(|| "unknown".into()),
            shell: value_for(output, "shell").unwrap_or_else(|| "cmd".into()),
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

    #[test]
    fn parses_powershell_probe_result() {
        let output = "\
home=C:\\Users\\deploy
pwd=C:\\srv\\app
user=deploy
platform=Windows
arch=x86_64
shell=powershell
cap_rg=1
cap_git=1
cap_patch=0
cap_python3=1
cap_grep=0
cap_find=1
cap_sed=0
cap_pwsh=1
cap_powershell=1";
        let probe = CapabilityProbe::parse_powershell(output, BackendRoute::SshExec);

        assert_eq!(probe.home, "C:\\Users\\deploy");
        assert_eq!(probe.cwd, "C:\\srv\\app");
        assert_eq!(probe.user, "deploy");
        assert_eq!(probe.platform, "Windows");
        assert_eq!(probe.arch, "x86_64");
        assert_eq!(probe.shell, "powershell");
        assert!(probe.capabilities.rg);
        assert!(probe.capabilities.git);
        assert!(!probe.capabilities.patch);
        assert!(probe.capabilities.python3);
        assert!(!probe.capabilities.grep);
        assert!(probe.capabilities.find);
        assert!(!probe.capabilities.sed);
        // PowerShell is always true for powershell probe
        assert!(probe.capabilities.powershell);
        assert_eq!(probe.capabilities.exec, true);
        assert_eq!(probe.capabilities.sftp, false);
        assert_eq!(probe.capabilities.pty, false);
    }

    #[test]
    fn parses_cmd_probe_result() {
        let output = "\
home=C:\\Users\\deploy
pwd=C:\\srv\\app
user=deploy
platform=Windows
arch=x86_64
shell=cmd
cap_rg=1
cap_git=1
cap_patch=0
cap_python3=0
cap_grep=0
cap_find=1
cap_sed=0
cap_pwsh=1
cap_powershell=0";
        let probe = CapabilityProbe::parse_cmd(output, BackendRoute::SshExec);

        assert_eq!(probe.home, "C:\\Users\\deploy");
        assert_eq!(probe.cwd, "C:\\srv\\app");
        assert_eq!(probe.user, "deploy");
        assert_eq!(probe.platform, "Windows");
        assert_eq!(probe.arch, "x86_64");
        assert_eq!(probe.shell, "cmd");
        assert!(probe.capabilities.rg);
        assert!(probe.capabilities.git);
        assert!(!probe.capabilities.patch);
        assert!(!probe.capabilities.python3);
        assert!(!probe.capabilities.grep);
        assert!(probe.capabilities.find);
        assert!(!probe.capabilities.sed);
        // powershell=true because pwsh is available (via has_cap OR)
        assert!(probe.capabilities.powershell);
        assert_eq!(probe.capabilities.exec, true);
        assert_eq!(probe.capabilities.sftp, false);
        assert_eq!(probe.capabilities.pty, false);
    }

    #[test]
    fn powershell_probe_defaults_when_empty() {
        let probe = CapabilityProbe::parse_powershell("", BackendRoute::Pty);

        assert_eq!(probe.home, "");
        assert_eq!(probe.cwd, "");
        assert_eq!(probe.user, "");
        assert_eq!(probe.platform, "Windows");
        assert_eq!(probe.arch, "unknown");
        assert_eq!(probe.shell, "powershell");
        assert!(!probe.capabilities.rg);
        assert!(!probe.capabilities.git);
        assert!(probe.capabilities.powershell); // always on for PS probe
    }

    #[test]
    fn cmd_probe_defaults_when_empty() {
        let probe = CapabilityProbe::parse_cmd("", BackendRoute::Pty);

        assert_eq!(probe.home, "");
        assert_eq!(probe.cwd, "");
        assert_eq!(probe.user, "");
        assert_eq!(probe.platform, "Windows");
        assert_eq!(probe.arch, "unknown");
        assert_eq!(probe.shell, "cmd");
        assert!(!probe.capabilities.rg);
        assert!(!probe.capabilities.git);
        assert!(!probe.capabilities.powershell); // empty output = no pwsh/powershell
    }
}
