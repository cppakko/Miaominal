use super::session::{ConnectionCancelled, SessionCommand, SessionEvent, SessionEventSender};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::forwarding::{AgentIdentitySummary, KbiChallenge, KbiPrompt};
use miaominal_core::profile::{AuthMethod, SessionProfile};
use miaominal_secrets::{SecretKind, SecretStore};
use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{
    Certificate, PrivateKey, PrivateKeyWithHashAlg, decode_secret_key, load_openssh_certificate,
    load_secret_key,
};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

pub(super) type LocalAgentTransport = Box<dyn AgentStream + Send + Unpin + 'static>;

#[derive(Debug)]
pub(crate) struct BridgeVaultLockedError;

impl std::fmt::Display for BridgeVaultLockedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("local vault is locked")
    }
}

impl std::error::Error for BridgeVaultLockedError {}

fn bridge_vault_locked() -> anyhow::Error {
    BridgeVaultLockedError.into()
}

pub async fn list_local_agent_identities() -> Result<Vec<AgentIdentitySummary>> {
    let mut agent = connect_local_agent().await?;
    let identities = agent
        .request_identities()
        .await
        .context("failed to enumerate local SSH agent identities")?;

    identities.iter().map(summarize_agent_identity).collect()
}

pub fn hydrate_profile_from_secrets(
    mut profile: SessionProfile,
    secrets: &SecretStore,
) -> SessionProfile {
    let needs_password = profile.password.is_empty() && profile.has_stored_password;
    let needs_passphrase = profile.passphrase.is_empty() && profile.has_stored_passphrase;

    if needs_password && needs_passphrase {
        match secrets.get_profile_secrets(&profile.id) {
            Ok(stored_secrets) => {
                match stored_secrets.password {
                    Some(secret) => profile.password = secret,
                    None => profile.has_stored_password = false,
                }

                match stored_secrets.passphrase {
                    Some(secret) => profile.passphrase = secret,
                    None => profile.has_stored_passphrase = false,
                }
            }
            Err(error) => log::warn!(
                "failed to load saved secrets for profile {}: {error:?}",
                profile.id
            ),
        }

        return profile;
    }

    if needs_password {
        match secrets.get(&profile.id, SecretKind::Password) {
            Ok(Some(secret)) => profile.password = secret,
            Ok(None) => profile.has_stored_password = false,
            Err(error) => log::warn!(
                "failed to load saved password for profile {}: {error:?}",
                profile.id
            ),
        }
    }

    if needs_passphrase {
        match secrets.get(&profile.id, SecretKind::Passphrase) {
            Ok(Some(secret)) => profile.passphrase = secret,
            Ok(None) => profile.has_stored_passphrase = false,
            Err(error) => log::warn!(
                "failed to load saved passphrase for profile {}: {error:?}",
                profile.id
            ),
        }
    }

    profile
}

pub(crate) fn validate_bridge_auth_material(
    profile: &SessionProfile,
    secrets: &SecretStore,
) -> Result<()> {
    match profile.effective_auth_method() {
        AuthMethod::Password | AuthMethod::KeyboardInteractive => {
            if profile.password.is_empty() {
                if profile.has_stored_password {
                    return Err(bridge_vault_locked());
                }
                bail!("SSH Bridge requires an available password for this profile");
            }
        }
        AuthMethod::KeyFile => {
            let path = profile.private_key_path.trim();
            if path.is_empty() {
                bail!("SSH key file authentication requires a private key path");
            }
            if !Path::new(path).is_file() {
                bail!("SSH private key file is unavailable: {path}");
            }
            if profile.passphrase.is_empty() && profile.has_stored_passphrase {
                return Err(bridge_vault_locked());
            }
        }
        AuthMethod::ManagedKey => {
            let id = profile.managed_key_id.trim();
            if id.is_empty() {
                bail!("managed key authentication requires a managed key id");
            }
            match secrets.get(id, SecretKind::ManagedPrivateKey) {
                Ok(Some(_)) => {}
                Ok(None) => bail!("managed key {id} is missing from the local credential store"),
                Err(error) if SecretStore::is_locked_error(&error) => {
                    return Err(bridge_vault_locked());
                }
                Err(error) => return Err(error),
            }
        }
        AuthMethod::Agent => {
            if profile.agent_identity.trim().is_empty() {
                bail!("SSH agent authentication requires an agent identity");
            }
        }
    }
    Ok(())
}

pub async fn authenticate<H>(
    session: &mut client::Handle<H>,
    profile: SessionProfile,
    secrets: &SecretStore,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    let auth_method = profile.effective_auth_method();
    let SessionProfile {
        username,
        password,
        has_stored_password,
        private_key_path,
        managed_key_id,
        agent_identity,
        certificate_path,
        passphrase,
        ..
    } = profile;

    match auth_method {
        AuthMethod::Password => {
            if password.is_empty() {
                if has_stored_password {
                    return Err(bridge_vault_locked());
                }
                bail!("password authentication requires a password");
            }

            let result = session
                .authenticate_password(username, password)
                .await
                .context("SSH password authentication failed")?;

            if !result.success() {
                bail!("password authentication was rejected by the server");
            }
        }
        AuthMethod::KeyFile => {
            if private_key_path.trim().is_empty() {
                bail!("SSH key file authentication requires a private key path");
            }

            let passphrase = (!passphrase.trim().is_empty()).then_some(passphrase.as_str());
            let key_pair = load_secret_key(Path::new(private_key_path.trim()), passphrase)
                .with_context(|| format!("failed to load private key {}", private_key_path))?;

            authenticate_with_private_key(session, username, certificate_path, key_pair).await?;
        }
        AuthMethod::ManagedKey => {
            if managed_key_id.trim().is_empty() {
                bail!("managed key authentication requires a managed key id");
            }

            let secret = match secrets.get(managed_key_id.trim(), SecretKind::ManagedPrivateKey) {
                Ok(Some(secret)) => secret,
                Ok(None) => {
                    bail!(
                        "managed key {} is missing from the local credential store",
                        managed_key_id
                    )
                }
                Err(error) if SecretStore::is_locked_error(&error) => {
                    return Err(bridge_vault_locked());
                }
                Err(error) => return Err(error),
            };
            let key_pair = decode_secret_key(&secret, None).with_context(|| {
                format!(
                    "failed to decode managed key {} from the local credential store",
                    managed_key_id
                )
            })?;
            authenticate_with_private_key(session, username, certificate_path, key_pair).await?;
        }
        AuthMethod::Agent => {
            if agent_identity.trim().is_empty() {
                bail!("SSH agent authentication requires an agent identity");
            }

            authenticate_with_agent(session, username, agent_identity, certificate_path).await?;
        }
        AuthMethod::KeyboardInteractive => {
            bail!(
                "keyboard-interactive authentication is not supported in this context; \
                 use a terminal session instead"
            );
        }
    }

    Ok(())
}

pub(crate) async fn authenticate_full<H>(
    session: &mut client::Handle<H>,
    profile: SessionProfile,
    secrets: &SecretStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    if profile.effective_auth_method() == AuthMethod::KeyboardInteractive {
        let username = profile.username.clone();
        authenticate_keyboard_interactive_flow(session, &username, command_receiver, event_sender)
            .await
    } else {
        authenticate(session, profile, secrets).await
    }
}

pub(crate) async fn authenticate_bridge<H>(
    session: &mut client::Handle<H>,
    profile: SessionProfile,
    secrets: &SecretStore,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    if profile.effective_auth_method() != AuthMethod::KeyboardInteractive {
        return authenticate(session, profile, secrets).await;
    }

    let username = profile.username.clone();
    let password = profile.password.clone();
    if password.is_empty() {
        if profile.has_stored_password {
            return Err(bridge_vault_locked());
        }
        bail!("keyboard-interactive authentication requires a saved password for SSH Bridge");
    }

    authenticate_bridge_keyboard_interactive(session, username, password).await
}

async fn authenticate_bridge_keyboard_interactive<H>(
    session: &mut client::Handle<H>,
    username: String,
    password: String,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut answered = false;
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None)
        .await
        .context("SSH keyboard-interactive authentication failed")?;

    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                bail!("keyboard-interactive authentication was rejected by the server");
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                validate_bridge_keyboard_interactive_prompts(
                    prompts.iter().map(|prompt| prompt.echo),
                    answered,
                )?;
                answered = true;
                response = session
                    .authenticate_keyboard_interactive_respond(vec![password.clone()])
                    .await
                    .context("SSH keyboard-interactive authentication response failed")?;
            }
        }
    }
}

fn validate_bridge_keyboard_interactive_prompts(
    echoes: impl IntoIterator<Item = bool>,
    already_answered: bool,
) -> Result<()> {
    let echoes = echoes.into_iter().collect::<Vec<_>>();
    if already_answered || echoes.len() != 1 || echoes[0] {
        bail!(
            "SSH Bridge only supports one non-echoing keyboard-interactive password prompt; \
             OTP and multi-question challenges are not supported"
        );
    }
    Ok(())
}

async fn authenticate_keyboard_interactive_flow<H>(
    session: &mut client::Handle<H>,
    username: &str,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut response = session
        .authenticate_keyboard_interactive_start(username.to_owned(), None)
        .await
        .context("SSH keyboard-interactive authentication failed")?;

    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                bail!("keyboard-interactive authentication was rejected by the server");
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let challenge = KbiChallenge {
                    name,
                    instructions,
                    prompts: prompts
                        .into_iter()
                        .map(|p| KbiPrompt {
                            prompt: p.prompt,
                            echo: p.echo,
                        })
                        .collect(),
                };
                if event_sender
                    .send(SessionEvent::KeyboardInteractivePrompt(challenge))
                    .await
                    .is_err()
                {
                    bail!("session event receiver is closed");
                }
                let answers = loop {
                    match command_receiver.recv().await {
                        Some(SessionCommand::KeyboardInteractiveResponse(answers)) => {
                            break answers;
                        }
                        Some(SessionCommand::Close) | None => {
                            return Err(ConnectionCancelled.into());
                        }
                        Some(_) => {}
                    }
                };
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .context("SSH keyboard-interactive authentication response failed")?;
            }
        }
    }
}

async fn authenticate_with_private_key<H>(
    session: &mut client::Handle<H>,
    username: String,
    certificate_path: String,
    key_pair: PrivateKey,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    if let Some(certificate) = load_profile_certificate(&certificate_path)? {
        let result = session
            .authenticate_openssh_cert(username, Arc::new(key_pair), certificate)
            .await
            .context("SSH public key certificate authentication failed")?;

        if !result.success() {
            bail!("public key certificate authentication was rejected by the server");
        }

        return Ok(());
    }

    let result = session
        .authenticate_publickey(
            username,
            PrivateKeyWithHashAlg::new(
                Arc::new(key_pair),
                session.best_supported_rsa_hash().await?.flatten(),
            ),
        )
        .await
        .context("SSH public key authentication failed")?;

    if !result.success() {
        bail!("public key authentication was rejected by the server");
    }

    Ok(())
}

async fn authenticate_with_agent<H>(
    session: &mut client::Handle<H>,
    username: String,
    agent_identity: String,
    certificate_path: String,
) -> Result<()>
where
    H: client::Handler<Error = anyhow::Error> + Send,
{
    let mut agent = connect_local_agent().await?;
    let identities = agent
        .request_identities()
        .await
        .context("failed to enumerate local SSH agent identities")?;
    let selected_identity = select_agent_identity(&identities, agent_identity.trim())?;
    let hash_alg = session.best_supported_rsa_hash().await?.flatten();

    let result = if let Some(certificate) = load_profile_certificate(&certificate_path)? {
        session
            .authenticate_certificate_with(username.clone(), certificate, hash_alg, &mut agent)
            .await
            .map_err(|error| anyhow!("SSH certificate authentication via agent failed: {error}"))?
    } else {
        match selected_identity {
            AgentIdentity::PublicKey { key, .. } => session
                .authenticate_publickey_with(username.clone(), key.clone(), hash_alg, &mut agent)
                .await
                .map_err(|error| {
                    anyhow!("SSH public key authentication via agent failed: {error}")
                })?,
            AgentIdentity::Certificate { certificate, .. } => session
                .authenticate_certificate_with(username, certificate.clone(), hash_alg, &mut agent)
                .await
                .map_err(|error| {
                    anyhow!("SSH certificate authentication via agent failed: {error}")
                })?,
        }
    };

    if !result.success() {
        bail!("agent-backed authentication was rejected by the server");
    }

    Ok(())
}

fn load_profile_certificate(certificate_path: &str) -> Result<Option<Certificate>> {
    if certificate_path.trim().is_empty() {
        return Ok(None);
    }

    let certificate = load_openssh_certificate(Path::new(certificate_path.trim()))
        .with_context(|| format!("failed to load certificate {}", certificate_path))?;
    Ok(Some(certificate))
}

fn select_agent_identity<'a>(
    identities: &'a [AgentIdentity],
    selected_identity: &str,
) -> Result<&'a AgentIdentity> {
    identities
        .iter()
        .find(|identity| match serialize_agent_identity(identity) {
            Ok(serialized) => serialized == selected_identity,
            Err(_) => false,
        })
        .ok_or_else(|| anyhow!("selected SSH agent identity is no longer available"))
}

fn serialize_agent_identity(identity: &AgentIdentity) -> Result<String> {
    match identity {
        AgentIdentity::PublicKey { key, .. } => key
            .to_openssh()
            .context("failed to serialize agent public key"),
        AgentIdentity::Certificate { certificate, .. } => certificate
            .to_openssh()
            .context("failed to serialize agent certificate"),
    }
}

fn summarize_agent_identity(identity: &AgentIdentity) -> Result<AgentIdentitySummary> {
    let serialized = serialize_agent_identity(identity)?;
    let kind = match identity {
        AgentIdentity::PublicKey { .. } => "Public key",
        AgentIdentity::Certificate { .. } => "Certificate",
    };
    let comment = identity.comment().to_string();
    let algorithm = serialized
        .split_whitespace()
        .next()
        .unwrap_or(kind)
        .to_string();
    let label = if comment.trim().is_empty() {
        format!("{kind}: {algorithm}")
    } else {
        comment.clone()
    };

    Ok(AgentIdentitySummary {
        serialized,
        label,
        comment,
        kind: kind.to_string(),
    })
}

pub(super) async fn connect_local_agent() -> Result<AgentClient<LocalAgentTransport>> {
    Ok(AgentClient::connect(connect_local_agent_stream().await?))
}

#[cfg(unix)]
pub(super) async fn connect_local_agent_stream() -> Result<LocalAgentTransport> {
    Ok(AgentClient::connect_env()
        .await
        .context("failed to connect to SSH_AUTH_SOCK")?
        .into_inner())
}

#[cfg(windows)]
pub(super) async fn connect_local_agent_stream() -> Result<LocalAgentTransport> {
    if let Ok(socket) = std::env::var("SSH_AUTH_SOCK")
        && !socket.trim().is_empty()
    {
        match AgentClient::connect_named_pipe(socket.trim()).await {
            Ok(client) => return Ok(client.into_inner()),
            Err(error) => {
                log::debug!("failed to connect SSH_AUTH_SOCK agent pipe: {error:?}");
            }
        }
    }

    match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(client) => return Ok(client.into_inner()),
        Err(error) => {
            log::debug!("failed to connect Windows OpenSSH agent pipe: {error:?}");
        }
    }

    AgentClient::connect_pageant()
        .await
        .map(|client| client.into_inner())
        .context("failed to connect to either the Windows OpenSSH agent pipe or Pageant")
}

#[cfg(not(any(unix, windows)))]
pub(super) async fn connect_local_agent_stream() -> Result<LocalAgentTransport> {
    bail!("local SSH agent access is not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::validate_bridge_keyboard_interactive_prompts;

    #[test]
    fn bridge_keyboard_interactive_accepts_one_hidden_prompt() {
        validate_bridge_keyboard_interactive_prompts([false], false)
            .expect("one hidden prompt should be accepted");
    }

    #[test]
    fn bridge_keyboard_interactive_rejects_complex_challenges() {
        for (echoes, already_answered) in [
            (Vec::<bool>::new(), false),
            (vec![true], false),
            (vec![false, false], false),
            (vec![false], true),
        ] {
            let error = validate_bridge_keyboard_interactive_prompts(echoes, already_answered)
                .expect_err("complex keyboard-interactive challenge should fail");
            assert!(error.to_string().contains("only supports one non-echoing"));
        }
    }
}
