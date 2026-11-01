//! Loopback API client used by the CLI.

use qf_engine::{DataDir, EngineConn};
use serde_json::Value;
use std::time::Duration;

pub struct Client {
    conn: EngineConn,
    agent: ureq::Agent,
}

#[derive(Debug)]
pub enum ClientError {
    NotRunning,
    Api { code: String, message: String },
    Transport(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning => write!(f, "the Quayflow engine is not running (quayflow start)"),
            ClientError::Api { message, .. } => write!(f, "{message}"),
            ClientError::Transport(m) => write!(f, "engine unreachable: {m}"),
        }
    }
}

impl Client {
    pub fn connect(dd: &DataDir) -> Result<Client, ClientError> {
        let conn = EngineConn::read(dd).ok_or(ClientError::NotRunning)?;
        if !qf_adapter::process::is_alive(conn.pid) {
            return Err(ClientError::NotRunning);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout(Duration::from_secs(120))
            .build();
        Ok(Client { conn, agent })
    }

    fn url(&self, route: &str) -> String {
        format!("http://127.0.0.1:{}{route}", self.conn.port)
    }

    fn handle(r: Result<ureq::Response, ureq::Error>) -> Result<Value, ClientError> {
        match r {
            Ok(resp) => resp.into_json().map_err(|e| ClientError::Transport(e.to_string())),
            Err(ureq::Error::Status(_, resp)) => {
                let v: Value = resp.into_json().unwrap_or_default();
                Err(ClientError::Api {
                    code: v["code"].as_str().unwrap_or("").into(),
                    message: v["error"].as_str().unwrap_or("request failed").into(),
                })
            }
            Err(ureq::Error::Transport(t)) => Err(ClientError::Transport(t.to_string())),
        }
    }

    pub fn get(&self, route: &str) -> Result<Value, ClientError> {
        Client::handle(
            self.agent
                .get(&self.url(route))
                .set("x-quayflow-token", &self.conn.token)
                .call(),
        )
    }

    pub fn post(&self, route: &str, body: Value) -> Result<Value, ClientError> {
        Client::handle(
            self.agent
                .post(&self.url(route))
                .set("x-quayflow-token", &self.conn.token)
                .set("content-type", "application/json")
                .send_string(&body.to_string()),
        )
    }

    pub fn pid(&self) -> u32 {
        self.conn.pid
    }

    /// The engine answers (not just a stale connection file).
    pub fn alive(&self) -> bool {
        self.agent
            .get(&self.url("/v1/compat"))
            .set("x-quayflow-token", &self.conn.token)
            .timeout(Duration::from_secs(5))
            .call()
            .is_ok()
    }
}
