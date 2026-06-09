use crate::error::{AgentError, AgentResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentJobId(pub String);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Exited,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPollResult {
    pub job_id: AgentJobId,
    pub status: JobStatus,
    pub exit_status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Default)]
pub struct AgentJobRegistry {
    jobs: Arc<Mutex<HashMap<AgentJobId, AgentJobRecord>>>,
}

struct AgentJobRecord {
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
        _command: impl Into<String>,
        marker: impl Into<String>,
    ) -> AgentJobId {
        let job_id = AgentJobId(Uuid::new_v4().to_string());
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(
                job_id.clone(),
                AgentJobRecord {
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
            .ok_or_else(|| AgentError::JobNotFound(job_id.0.clone()))
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
        let job_id = registry.insert_remote_job("sleep 1", "/tmp/job");

        assert!(registry.contains(&job_id));
        assert_eq!(registry.remote_marker(&job_id).unwrap(), "/tmp/job");
        registry.remove(&job_id).unwrap();
        assert!(matches!(
            registry.remote_marker(&job_id),
            Err(AgentError::JobNotFound(_))
        ));
    }
}
