use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use miaominal_core::proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol};
use miaominal_secrets::{SecretKind, SecretStore};
use std::{
    io::Cursor,
    net::IpAddr,
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::timeout;
use tokio_socks::tcp::Socks5Stream;

const MAX_HTTP_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;
const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> Transport for T where T: AsyncRead + AsyncWrite + Unpin + Send {}
pub type BoxedTransport = Box<dyn Transport>;

pub fn resolve_entry_proxy<'a>(
    proxy_id: Option<&str>,
    proxies: &'a [ProxyProfile],
) -> Result<Option<&'a ProxyProfile>> {
    proxy_id
        .map(|proxy_id| {
            proxies
                .iter()
                .find(|proxy| proxy.id == proxy_id)
                .ok_or_else(|| anyhow!("configured entry proxy {proxy_id} is no longer available"))
        })
        .transpose()
}

pub async fn connect_via_proxy(
    proxy: &ProxyProfile,
    target_host: &str,
    target_port: u16,
    secrets: &SecretStore,
) -> Result<BoxedTransport> {
    let password = match proxy.auth_mode {
        ProxyAuthMode::None => None,
        ProxyAuthMode::UsernamePassword => Some(
            secrets
                .get(&proxy.id, SecretKind::ProxyPassword)
                .with_context(|| {
                    format!(
                        "failed to read password for proxy {}",
                        proxy.connection_label()
                    )
                })?
                .ok_or_else(|| {
                    anyhow!(
                        "proxy {} requires a saved password",
                        proxy.connection_label()
                    )
                })?,
        ),
    };

    connect_via_proxy_with_timeout(
        proxy,
        target_host,
        target_port,
        password.as_deref(),
        PROXY_CONNECT_TIMEOUT,
    )
    .await
}

async fn connect_via_proxy_with_timeout(
    proxy: &ProxyProfile,
    target_host: &str,
    target_port: u16,
    password: Option<&str>,
    timeout_duration: Duration,
) -> Result<BoxedTransport> {
    let connect_future = async {
        match proxy.protocol {
            ProxyProtocol::Socks5 => {
                connect_socks5(proxy, target_host, target_port, password).await
            }
            ProxyProtocol::HttpConnect => {
                connect_http(proxy, target_host, target_port, password).await
            }
        }
    };

    timeout(timeout_duration, connect_future)
        .await
        .map_err(|_| {
            anyhow!(
                "proxy {} connection or handshake timed out after {} seconds",
                proxy.connection_label(),
                timeout_duration.as_secs()
            )
        })?
}

async fn resolve_proxy_addresses(proxy: &ProxyProfile) -> Result<Vec<std::net::SocketAddr>> {
    let proxy_host = unbracket_host(&proxy.host);
    let addresses = lookup_host((proxy_host, proxy.port))
        .await
        .with_context(|| {
            format!(
                "failed to resolve proxy {} at {}:{}",
                proxy.connection_label(),
                proxy.host,
                proxy.port
            )
        })?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        bail!(
            "proxy {} at {}:{} resolved to no addresses",
            proxy.connection_label(),
            proxy.host,
            proxy.port
        );
    }
    Ok(addresses)
}

async fn connect_socks5(
    proxy: &ProxyProfile,
    target_host: &str,
    target_port: u16,
    password: Option<&str>,
) -> Result<BoxedTransport> {
    let proxy_addresses = resolve_proxy_addresses(proxy).await?;
    let proxy_address = proxy_addresses.as_slice();
    let target_host_unbracketed = unbracket_host(target_host);

    if proxy.resolve_dns_through_proxy {
        let stream = match password {
            Some(password) => {
                Socks5Stream::connect_with_password(
                    proxy_address,
                    (target_host_unbracketed, target_port),
                    &proxy.username,
                    password,
                )
                .await
            }
            None => {
                Socks5Stream::connect(proxy_address, (target_host_unbracketed, target_port)).await
            }
        }
        .with_context(|| {
            format!(
                "SOCKS5 proxy {} failed to connect to {}:{}",
                proxy.connection_label(),
                target_host,
                target_port
            )
        })?;
        return Ok(Box::new(stream));
    }

    let target_addresses = lookup_host((target_host_unbracketed, target_port))
        .await
        .with_context(|| format!("failed to resolve proxy target {target_host}:{target_port}"))?
        .collect::<Vec<_>>();
    if target_addresses.is_empty() {
        bail!("proxy target {target_host}:{target_port} resolved to no addresses");
    }

    let mut last_error = None;
    for target_address in target_addresses {
        let result = match password {
            Some(password) => {
                Socks5Stream::connect_with_password(
                    proxy_address,
                    target_address,
                    &proxy.username,
                    password,
                )
                .await
            }
            None => Socks5Stream::connect(proxy_address, target_address).await,
        };
        match result {
            Ok(stream) => return Ok(Box::new(stream)),
            Err(error) => last_error = Some(error),
        }
    }

    Err(anyhow!(
        "SOCKS5 proxy {} failed to connect to {}:{}: {}",
        proxy.connection_label(),
        target_host,
        target_port,
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown proxy error".into())
    ))
}

async fn connect_http(
    proxy: &ProxyProfile,
    target_host: &str,
    target_port: u16,
    password: Option<&str>,
) -> Result<BoxedTransport> {
    let mut stream = TcpStream::connect((unbracket_host(&proxy.host), proxy.port))
        .await
        .with_context(|| {
            format!(
                "failed to connect to HTTP proxy {} at {}:{}",
                proxy.connection_label(),
                proxy.host,
                proxy.port
            )
        })?;
    let authority = format_authority(target_host, target_port)?;
    let mut request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if let Some(password) = password {
        let credentials = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{password}", proxy.username));
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(&credentials);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .with_context(|| {
            format!(
                "failed to send CONNECT request to {}",
                proxy.connection_label()
            )
        })?;
    stream.flush().await.with_context(|| {
        format!(
            "failed to flush CONNECT request to {}",
            proxy.connection_label()
        )
    })?;

    let mut response_bytes = Vec::with_capacity(1024);
    let header_len = loop {
        if response_bytes.len() >= MAX_HTTP_CONNECT_RESPONSE_BYTES {
            bail!(
                "HTTP proxy {} returned response headers larger than {} bytes",
                proxy.connection_label(),
                MAX_HTTP_CONNECT_RESPONSE_BYTES
            );
        }
        let mut chunk = [0u8; 1024];
        let read = stream.read(&mut chunk).await.with_context(|| {
            format!(
                "failed to read CONNECT response from {}",
                proxy.connection_label()
            )
        })?;
        if read == 0 {
            bail!(
                "HTTP proxy {} closed before completing CONNECT",
                proxy.connection_label()
            );
        }
        response_bytes.extend_from_slice(&chunk[..read]);

        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        match response
            .parse(&response_bytes)
            .context("failed to parse HTTP CONNECT response")?
        {
            httparse::Status::Partial => continue,
            httparse::Status::Complete(header_len) => {
                let status = response
                    .code
                    .ok_or_else(|| anyhow!("HTTP CONNECT response has no status code"))?;
                if status == 407 {
                    bail!(
                        "HTTP proxy {} rejected proxy authentication",
                        proxy.connection_label()
                    );
                }
                if !(200..300).contains(&status) {
                    bail!(
                        "HTTP proxy {} rejected CONNECT with status {}",
                        proxy.connection_label(),
                        status
                    );
                }
                break header_len;
            }
        }
    };

    let prefix = response_bytes.split_off(header_len);
    Ok(Box::new(PrefixedTcpStream {
        prefix: Cursor::new(prefix),
        stream,
    }))
}

fn format_authority(host: &str, port: u16) -> Result<String> {
    let host = unbracket_host(host);
    if host.is_empty()
        || host.chars().any(char::is_control)
        || host.chars().any(char::is_whitespace)
    {
        bail!("proxy target host contains invalid characters");
    }
    match host.parse::<IpAddr>() {
        Ok(address) if address.is_ipv6() => Ok(format!("[{host}]:{port}")),
        Ok(_) => Ok(format!("{host}:{port}")),
        Err(_) if host.contains(':') => {
            bail!("proxy target host contains an invalid colon")
        }
        Err(_) => Ok(format!("{host}:{port}")),
    }
}

fn unbracket_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

struct PrefixedTcpStream {
    prefix: Cursor<Vec<u8>>,
    stream: TcpStream,
}

impl AsyncRead for PrefixedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let position = self.prefix.position() as usize;
        let prefix = self.prefix.get_ref();
        if position < prefix.len() && buffer.remaining() > 0 {
            let count = (prefix.len() - position).min(buffer.remaining());
            buffer.put_slice(&prefix[position..position + count]);
            self.prefix.set_position((position + count) as u64);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for PrefixedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn http_proxy(port: u16) -> ProxyProfile {
        ProxyProfile {
            id: "proxy-http".into(),
            name: "HTTP test proxy".into(),
            protocol: ProxyProtocol::HttpConnect,
            host: "127.0.0.1".into(),
            port,
            auth_mode: ProxyAuthMode::None,
            username: String::new(),
            resolve_dns_through_proxy: false,
            has_stored_password: false,
        }
    }

    fn socks_proxy(port: u16, auth_mode: ProxyAuthMode, remote_dns: bool) -> ProxyProfile {
        ProxyProfile {
            id: "proxy-socks".into(),
            name: "SOCKS test proxy".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port,
            auth_mode,
            username: "alice".into(),
            resolve_dns_through_proxy: remote_dns,
            has_stored_password: auth_mode == ProxyAuthMode::UsernamePassword,
        }
    }

    async fn spawn_http_proxy(response: Vec<u8>) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("HTTP test proxy should bind");
        let port = listener.local_addr().expect("listener has address").port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("proxy should accept");
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8; 512];
                let read = stream.read(&mut chunk).await.expect("request should read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = stream.write_all(&response).await;
            request
        });
        (port, task)
    }

    async fn spawn_stalled_http_proxy() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("HTTP test proxy should bind");
        let port = listener.local_addr().expect("listener has address").port();
        let task = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("proxy should accept");
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        (port, task)
    }

    async fn spawn_socks_proxy(
        credentials: Option<(&'static str, &'static str)>,
        reject_method: bool,
    ) -> (u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("SOCKS test proxy should bind");
        let port = listener.local_addr().expect("listener has address").port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("proxy should accept");
            let mut greeting = [0u8; 2];
            stream
                .read_exact(&mut greeting)
                .await
                .expect("SOCKS greeting should read");
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0u8; greeting[1] as usize];
            stream
                .read_exact(&mut methods)
                .await
                .expect("SOCKS methods should read");
            if reject_method {
                stream
                    .write_all(&[5, 0xff])
                    .await
                    .expect("method rejection should write");
                return String::new();
            }

            let method = if credentials.is_some() { 2 } else { 0 };
            assert!(methods.contains(&method));
            stream
                .write_all(&[5, method])
                .await
                .expect("method selection should write");
            if let Some((expected_username, expected_password)) = credentials {
                let mut auth_header = [0u8; 2];
                stream
                    .read_exact(&mut auth_header)
                    .await
                    .expect("auth header should read");
                assert_eq!(auth_header[0], 1);
                let mut username = vec![0u8; auth_header[1] as usize];
                stream
                    .read_exact(&mut username)
                    .await
                    .expect("username should read");
                let password_len = stream.read_u8().await.expect("password length should read");
                let mut password = vec![0u8; password_len as usize];
                stream
                    .read_exact(&mut password)
                    .await
                    .expect("password should read");
                assert_eq!(username, expected_username.as_bytes());
                assert_eq!(password, expected_password.as_bytes());
                stream
                    .write_all(&[1, 0])
                    .await
                    .expect("auth result should write");
            }

            let mut request = [0u8; 4];
            stream
                .read_exact(&mut request)
                .await
                .expect("SOCKS request should read");
            assert_eq!(&request[..3], &[5, 1, 0]);
            let target = match request[3] {
                1 => {
                    let mut address = [0u8; 4];
                    stream
                        .read_exact(&mut address)
                        .await
                        .expect("IPv4 should read");
                    std::net::Ipv4Addr::from(address).to_string()
                }
                3 => {
                    let len = stream.read_u8().await.expect("domain length should read");
                    let mut domain = vec![0u8; len as usize];
                    stream
                        .read_exact(&mut domain)
                        .await
                        .expect("domain should read");
                    String::from_utf8(domain).expect("domain should be UTF-8")
                }
                4 => {
                    let mut address = [0u8; 16];
                    stream
                        .read_exact(&mut address)
                        .await
                        .expect("IPv6 should read");
                    std::net::Ipv6Addr::from(address).to_string()
                }
                atyp => panic!("unexpected SOCKS address type {atyp}"),
            };
            let mut port = [0u8; 2];
            stream
                .read_exact(&mut port)
                .await
                .expect("port should read");
            assert_eq!(u16::from_be_bytes(port), 22);
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .expect("SOCKS success should write");
            target
        });
        (port, task)
    }

    #[test]
    fn formats_ipv6_authority_with_brackets() {
        assert_eq!(format_authority("::1", 22).unwrap(), "[::1]:22");
        assert_eq!(format_authority("[::1]", 22).unwrap(), "[::1]:22");
    }

    #[test]
    fn formats_domain_authority() {
        assert_eq!(
            format_authority("example.com", 22).unwrap(),
            "example.com:22"
        );
    }

    #[test]
    fn rejects_non_ip_authority_with_colon() {
        let error =
            format_authority("example.com:2222", 22).expect_err("embedded port should be rejected");
        assert!(error.to_string().contains("colon"));
    }

    #[test]
    fn missing_entry_proxy_is_a_hard_error() {
        let error = resolve_entry_proxy(Some("missing"), &[])
            .expect_err("missing proxy should fail instead of falling back");
        assert!(error.to_string().contains("no longer available"));
    }

    #[tokio::test]
    async fn http_connect_preserves_coalesced_ssh_banner_and_sends_basic_auth() {
        let response =
            b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: test\r\n\r\nSSH-2.0-test\r\n"
                .to_vec();
        let (port, request_task) = spawn_http_proxy(response).await;
        let mut proxy = http_proxy(port);
        proxy.auth_mode = ProxyAuthMode::UsernamePassword;
        proxy.username = "alice".into();
        let mut transport = connect_http(&proxy, "2001:db8::1", 22, Some("secret"))
            .await
            .expect("CONNECT should succeed");
        let mut banner = vec![0u8; b"SSH-2.0-test\r\n".len()];
        transport
            .read_exact(&mut banner)
            .await
            .expect("coalesced SSH banner should remain readable");
        assert_eq!(banner, b"SSH-2.0-test\r\n");

        let request = String::from_utf8(request_task.await.expect("proxy task should finish"))
            .expect("request should be UTF-8");
        assert!(request.starts_with("CONNECT [2001:db8::1]:22 HTTP/1.1\r\n"));
        assert!(request.contains("Host: [2001:db8::1]:22\r\n"));
        assert!(request.contains("Proxy-Authorization: Basic YWxpY2U6c2VjcmV0\r\n"));
    }

    #[tokio::test]
    async fn http_connect_maps_407_to_authentication_error() {
        let (port, _) = spawn_http_proxy(
            b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n".to_vec(),
        )
        .await;
        let error = connect_http(&http_proxy(port), "example.com", 22, None)
            .await
            .err()
            .expect("407 should fail");
        assert!(error.to_string().contains("authentication"));
    }

    #[tokio::test]
    async fn proxy_connect_timeout_bounds_stalled_handshake() {
        let (port, task) = spawn_stalled_http_proxy().await;
        let error = connect_via_proxy_with_timeout(
            &http_proxy(port),
            "example.com",
            22,
            None,
            Duration::from_millis(20),
        )
        .await
        .err()
        .expect("stalled CONNECT should time out");
        assert!(error.to_string().contains("timed out"));
        task.abort();
    }

    #[tokio::test]
    async fn http_connect_rejects_non_success_malformed_and_oversized_responses() {
        let mut oversized = b"HTTP/1.1 200 OK\r\nX-Long: ".to_vec();
        oversized.extend(std::iter::repeat_n(b'x', MAX_HTTP_CONNECT_RESPONSE_BYTES));
        for (response, expected) in [
            (b"HTTP/1.1 503 Unavailable\r\n\r\n".to_vec(), "status 503"),
            (b"not-http\r\n\r\n".to_vec(), "parse"),
            (oversized, "larger"),
        ] {
            let (port, _) = spawn_http_proxy(response).await;
            let error = connect_http(&http_proxy(port), "example.com", 22, None)
                .await
                .err()
                .expect("invalid CONNECT response should fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} in {error:#}"
            );
        }
    }

    #[tokio::test]
    async fn socks5_remote_dns_sends_domain_name() {
        let (port, target_task) = spawn_socks_proxy(None, false).await;
        connect_socks5(
            &socks_proxy(port, ProxyAuthMode::None, true),
            "ssh.example",
            22,
            None,
        )
        .await
        .expect("SOCKS connection should succeed");
        assert_eq!(
            target_task.await.expect("proxy task should finish"),
            "ssh.example"
        );
    }

    #[tokio::test]
    async fn resolves_socks5_proxy_hostname_asynchronously() {
        let mut proxy = socks_proxy(1080, ProxyAuthMode::None, true);
        proxy.host = "localhost".into();
        let addresses = resolve_proxy_addresses(&proxy)
            .await
            .expect("localhost should resolve");
        assert!(!addresses.is_empty());
    }

    #[tokio::test]
    async fn socks5_remote_dns_unbrackets_ipv6_targets() {
        let (port, target_task) = spawn_socks_proxy(None, false).await;
        connect_socks5(
            &socks_proxy(port, ProxyAuthMode::None, true),
            "[::1]",
            22,
            None,
        )
        .await
        .expect("SOCKS connection should succeed");
        assert_eq!(target_task.await.expect("proxy task should finish"), "::1");
    }

    #[tokio::test]
    async fn socks5_username_password_authentication_is_used() {
        let (port, target_task) = spawn_socks_proxy(Some(("alice", "secret")), false).await;
        connect_socks5(
            &socks_proxy(port, ProxyAuthMode::UsernamePassword, true),
            "ssh.example",
            22,
            Some("secret"),
        )
        .await
        .expect("authenticated SOCKS connection should succeed");
        assert_eq!(
            target_task.await.expect("proxy task should finish"),
            "ssh.example"
        );
    }

    #[tokio::test]
    async fn socks5_local_dns_sends_ip_and_method_rejection_fails() {
        let (port, target_task) = spawn_socks_proxy(None, false).await;
        connect_socks5(
            &socks_proxy(port, ProxyAuthMode::None, false),
            "127.0.0.1",
            22,
            None,
        )
        .await
        .expect("locally resolved SOCKS connection should succeed");
        assert_eq!(
            target_task.await.expect("proxy task should finish"),
            "127.0.0.1"
        );

        let (port, _) = spawn_socks_proxy(None, true).await;
        let error = connect_socks5(
            &socks_proxy(port, ProxyAuthMode::None, true),
            "ssh.example",
            22,
            None,
        )
        .await
        .err()
        .expect("method rejection should fail");
        assert!(!error.to_string().is_empty());
    }
}
