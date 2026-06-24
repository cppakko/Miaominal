use anyhow::{Result, anyhow};
use miaominal_agent::{
    AgentExecChannel, AgentJobRegistry, AgentToolCallRequest, AgentToolCallResponse,
};
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretKind;
use miaominal_secrets::SecretStore;
use miaominal_settings::WebSearchConfig;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use tokio::runtime::Handle as TokioHandle;

#[derive(Clone)]
pub struct AgentService {
    runtime: TokioHandle,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    jobs: AgentJobRegistry,
    web_search: WebSearchConfig,
}

impl AgentService {
    pub fn new(runtime: TokioHandle, secrets: SecretStore, known_hosts: KnownHostsStore) -> Self {
        Self {
            runtime,
            secrets,
            known_hosts,
            jobs: AgentJobRegistry::new(),
            web_search: WebSearchConfig::default(),
        }
    }

    pub fn with_web_search(mut self, web_search: WebSearchConfig) -> Self {
        self.web_search = web_search;
        self
    }

    pub fn channel_for_profile(
        &self,
        profile_id: &str,
        sessions: &[SessionProfile],
    ) -> Result<AgentExecChannel> {
        let profile = sessions
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| anyhow!("profile `{profile_id}` was not found"))?;

        let mut channel = AgentExecChannel::for_profile_with_jobs(
            profile,
            sessions.to_vec(),
            self.secrets.clone(),
            self.known_hosts.clone(),
            self.jobs.clone(),
        );
        if self.web_search.enabled {
            let web_search_api_key = self
                .secrets
                .get("web_search", SecretKind::WebSearchApiKey)
                .unwrap_or_else(|error| {
                    log::warn!("failed to load web search API key: {error:?}");
                    None
                });
            channel = channel.with_web_search_config(self.web_search.clone(), web_search_api_key);
        }

        Ok(channel)
    }

    pub fn call_tool(
        &self,
        profile_id: &str,
        sessions: &[SessionProfile],
        request: AgentToolCallRequest,
    ) -> Result<AgentToolCallResponse> {
        let channel = self.channel_for_profile(profile_id, sessions)?;
        self.runtime
            .block_on(channel.call_tool(request))
            .map_err(anyhow::Error::from)
    }

    pub fn jobs(&self) -> AgentJobRegistry {
        self.jobs.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{SessionProfile, ShellType};

    fn service() -> AgentService {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should start");
        let handle = runtime.handle().clone();
        std::mem::forget(runtime);
        AgentService::new(
            handle,
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-service-known-hosts")),
        )
    }

    fn profile(id: &str, shell_type: ShellType) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = shell_type;
        profile
    }

    #[test]
    fn missing_profile_returns_error() {
        let service = service();

        let error = match service
            .channel_for_profile("missing", &[profile("session-1", ShellType::Posix)])
        {
            Ok(_) => panic!("missing profile should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("was not found"));
    }

    #[test]
    fn powershell_profile_is_accepted() {
        let service = service();

        service
            .channel_for_profile("session-1", &[profile("session-1", ShellType::PowerShell)])
            .expect("PowerShell profile should be accepted");
    }

    #[test]
    fn cmd_profile_is_accepted() {
        let service = service();

        service
            .channel_for_profile("session-1", &[profile("session-1", ShellType::Cmd)])
            .expect("Cmd profile should be accepted");
    }
}
