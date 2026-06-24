use crate::error::{AgentError, AgentResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentJobId(pub String);

impl AgentJobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn remote_marker(&self) -> AgentResult<String> {
        Uuid::from_str(&self.0)
            .map_err(|_| AgentError::JobNotFound(self.0.clone()))
            .map(|uuid| format!("/tmp/miaominal-agent-{uuid}.status"))
    }
}

impl Default for AgentJobId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Exited,
    Stopped,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPollResult {
    pub job_id: AgentJobId,
    pub status: JobStatus,
    pub exit_status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentJobSummary {
    pub job_id: AgentJobId,
    pub command: String,
}

#[derive(Clone, Default)]
pub struct AgentJobRegistry {
    jobs: Arc<Mutex<HashMap<AgentJobId, AgentJobRecord>>>,
}

struct AgentJobRecord {
    command: String,
    marker: String,
}

impl AgentJobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, job_id: &AgentJobId) -> bool {
        self.jobs
            .lock()
            .map(|jobs| jobs.contains_key(job_id))
            .unwrap_or(false)
    }

    pub fn insert_remote_job(
        &self,
        command: impl Into<String>,
        marker: impl Into<String>,
    ) -> AgentJobId {
        let job_id = AgentJobId::new();
        self.insert_remote_job_with_id(job_id, command, marker)
    }

    pub fn insert_remote_job_with_id(
        &self,
        job_id: AgentJobId,
        command: impl Into<String>,
        marker: impl Into<String>,
    ) -> AgentJobId {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(
                job_id.clone(),
                AgentJobRecord {
                    command: command.into(),
                    marker: marker.into(),
                },
            );
        }
        job_id
    }

    pub fn remote_marker(&self, job_id: &AgentJobId) -> AgentResult<String> {
        self.jobs
            .lock()
            .map_err(|_| AgentError::Backend(anyhow::anyhow!("job registry is poisoned")))?
            .get(job_id)
            .map(|record| record.marker.clone())
            .map_or_else(|| job_id.remote_marker(), Ok)
    }

    pub fn list(&self) -> AgentResult<Vec<AgentJobSummary>> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| AgentError::Backend(anyhow::anyhow!("job registry is poisoned")))?
            .iter()
            .map(|(job_id, record)| AgentJobSummary {
                job_id: job_id.clone(),
                command: record.command.clone(),
            })
            .collect::<Vec<_>>();
        jobs.sort_by(|left, right| left.job_id.0.cmp(&right.job_id.0));
        Ok(jobs)
    }

    pub fn remove(&self, job_id: &AgentJobId) -> AgentResult<()> {
        self.jobs
            .lock()
            .map_err(|_| AgentError::Backend(anyhow::anyhow!("job registry is poisoned")))?
            .remove(job_id)
            .map(|_| ())
            .ok_or_else(|| AgentError::JobNotFound(job_id.0.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_tracks_and_removes_remote_jobs() {
        let registry = AgentJobRegistry::new();
        let job_id = AgentJobId::new();
        registry.insert_remote_job_with_id(
            job_id.clone(),
            "sleep 1",
            job_id.remote_marker().unwrap(),
        );

        assert!(registry.contains(&job_id));
        assert_eq!(
            registry.remote_marker(&job_id).unwrap(),
            format!("/tmp/miaominal-agent-{}.status", job_id.0)
        );
        registry.remove(&job_id).unwrap();
        assert_eq!(
            registry.remote_marker(&job_id).unwrap(),
            format!("/tmp/miaominal-agent-{}.status", job_id.0)
        );
    }

    #[test]
    fn registry_can_recover_marker_from_generated_job_id() {
        let registry = AgentJobRegistry::new();
        let job_id = AgentJobId::new();

        assert_eq!(
            registry.remote_marker(&job_id).unwrap(),
            format!("/tmp/miaominal-agent-{}.status", job_id.0)
        );
    }

    #[test]
    fn registry_rejects_untrusted_job_id_paths() {
        let registry = AgentJobRegistry::new();
        let job_id = AgentJobId("../../etc/passwd".into());

        assert!(matches!(
            registry.remote_marker(&job_id),
            Err(AgentError::JobNotFound(_))
        ));
    }
}
