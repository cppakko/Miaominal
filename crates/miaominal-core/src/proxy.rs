use serde::{Deserialize, Serialize};

pub const DEFAULT_SOCKS5_PROXY_PORT: u16 = 1080;
pub const DEFAULT_HTTP_CONNECT_PROXY_PORT: u16 = 8080;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyProtocol {
    #[default]
    Socks5,
    HttpConnect,
}

impl ProxyProtocol {
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Socks5 => DEFAULT_SOCKS5_PROXY_PORT,
            Self::HttpConnect => DEFAULT_HTTP_CONNECT_PROXY_PORT,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyAuthMode {
    #[default]
    None,
    UsernamePassword,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub auth_mode: ProxyAuthMode,
    #[serde(default)]
    pub username: String,
    #[serde(default = "default_resolve_dns_through_proxy")]
    pub resolve_dns_through_proxy: bool,
    #[serde(default)]
    pub has_stored_password: bool,
}

const fn default_resolve_dns_through_proxy() -> bool {
    true
}

impl ProxyProfile {
    pub fn blank(id: impl Into<String>, ordinal: usize) -> Self {
        Self {
            id: id.into(),
            name: format!("Proxy {ordinal}"),
            protocol: ProxyProtocol::Socks5,
            host: String::new(),
            port: DEFAULT_SOCKS5_PROXY_PORT,
            auth_mode: ProxyAuthMode::None,
            username: String::new(),
            resolve_dns_through_proxy: true,
            has_stored_password: false,
        }
    }

    pub fn connection_label(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            self.name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_proxy_uses_socks_defaults() {
        let proxy = ProxyProfile::blank("proxy-1", 1);

        assert_eq!(proxy.protocol, ProxyProtocol::Socks5);
        assert_eq!(proxy.port, DEFAULT_SOCKS5_PROXY_PORT);
        assert!(proxy.resolve_dns_through_proxy);
    }

    #[test]
    fn legacy_proxy_defaults_to_remote_dns() {
        let proxy: ProxyProfile = serde_json::from_str(
            r#"{"id":"proxy-1","name":"Local","host":"127.0.0.1","port":1080}"#,
        )
        .expect("proxy should deserialize");

        assert!(proxy.resolve_dns_through_proxy);
        assert_eq!(proxy.auth_mode, ProxyAuthMode::None);
    }
}
