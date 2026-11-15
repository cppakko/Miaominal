use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::toolkit::{ChannelToolRuntime, ToolInvocation, ToolRuntime as _};
use crate::transcript::ToolResultStatus;
use crate::{AgentExecChannel, AgentToolCancellation};
use anyhow::Result;
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;

const PATCH: &str = "--- new.txt\n+++ new.txt\n@@ -1,5 +1,5 @@\n listen 8080;\n-server_name example.com;\n+server_name example.com www.example.com;\n root /var/www/site;\n index index.html;\n     add_header X-Frame-Options DENY;\n";

struct ExecServer {
    commands: Arc<Mutex<Vec<String>>>,
}

impl server::Handler for ExecServer {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth> {
        Ok(if user == "fixture" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<()> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<()> {
        self.commands
            .lock()
            .unwrap()
            .push(String::from_utf8(data.to_vec())?);
        session.channel_success(channel)?;
        session.data(channel, b"patching file new.txt\n".to_vec())?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

#[test]
fn patch_tool_completes_ssh_exec_with_bounded_worker_stack() {
    // Leave headroom inside a normal 2 MiB worker for the rest of the agent.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(patch_tool_completes_ssh_exec());
}

async fn patch_tool_completes_ssh_exec() {
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", port, key.public_key())
        .unwrap();
    let config = Arc::new(server::Config {
        keys: vec![key],
        ..Default::default()
    });
    let commands = Arc::new(Mutex::new(Vec::new()));
    let server_commands = commands.clone();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let config = config.clone();
            let commands = server_commands.clone();
            tokio::spawn(async move {
                let session = server::run_stream(config, stream, ExecServer { commands })
                    .await
                    .unwrap();
                let _ = session.await;
            });
        }
    });
    let mut profile = SessionProfile::blank("patch-stack", 1);
    profile.host = "127.0.0.1".into();
    profile.port = port;
    profile.username = "fixture".into();
    profile.password = "secret".into();
    // The fixture has only an exec server. Keep SFTP authorization out of this
    // transport test and exercise the real SSH handshake and patch dispatch.
    let channel = AgentExecChannel::for_profile(
        profile,
        vec![],
        SecretStore::new_locked_vault(),
        known_hosts,
    )
    .with_policy_bypass(true);
    let runtime = ChannelToolRuntime::new(Some(channel), "fixture", None);
    let (progress, _) = tokio::sync::mpsc::unbounded_channel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.execute(
            ToolInvocation {
                call_id: "patch-stack".into(),
                name: "apply_patch".into(),
                arguments: serde_json::json!({ "patch": PATCH, "base_dir": "/tmp/difftest" }),
            },
            AgentToolCancellation::new(),
            progress,
        ),
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(outcome.status, ToolResultStatus::Ok, "{outcome:?}");
    let commands = commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert!(commands[0].contains("patch -p0"));
    assert!(commands[0].contains(PATCH));
}
