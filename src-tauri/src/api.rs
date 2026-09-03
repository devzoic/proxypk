use crate::models::*;
use reqwest::Client;
use std::time::Duration;

/// HTTP client for communicating with the Laravel server API.
#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
    machine_id: String,
}

impl ApiClient {
    pub fn new(base_url: &str, machine_id: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: None,
            machine_id: machine_id.to_string(),
        }
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(ref token) = self.token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", token).parse().unwrap(),
            );
        }
        headers.insert(
            reqwest::header::ACCEPT,
            "application/json".parse().unwrap(),
        );
        headers
    }

    /// Register this device with the server.
    pub async fn register(
        &self,
        hostname: &str,
        os_type: &str,
        os_version: &str,
    ) -> Result<RegisterResponse, String> {
        let url = format!("{}/api/desktop/register", self.base_url);
        let body = RegisterRequest {
            machine_id: self.machine_id.clone(),
            hostname: hostname.to_string(),
            os_type: os_type.to_string(),
            os_version: os_version.to_string(),
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if !status.is_success() {
            return Err(format!("Registration failed (HTTP {}): {}", status, text));
        }

        serde_json::from_str::<RegisterResponse>(&text)
            .map_err(|e| format!("JSON decode error ({}): {}", e, text))
    }

    /// Send heartbeat to server.
    pub async fn heartbeat(&self) -> Result<HeartbeatResponse, String> {
        let url = format!("{}/api/desktop/heartbeat", self.base_url);
        let body = HeartbeatRequest {
            machine_id: self.machine_id.clone(),
            public_ip: None,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            cpu_usage: None,
            ram_usage: None,
            active_connections: None,
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Heartbeat error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read heartbeat response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Heartbeat failed (HTTP {}): {}", status, text));
        }

        serde_json::from_str::<HeartbeatResponse>(&text)
            .map_err(|e| format!("JSON decode error ({}): {}", e, text))
    }

    /// Sync network adapters.
    pub async fn sync_adapters(
        &self,
        adapters: Vec<AdapterInfo>,
    ) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/desktop/adapters/sync", self.base_url);
        let body = AdapterSyncRequest {
            machine_id: self.machine_id.clone(),
            adapters,
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Sync network error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read sync response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Sync failed (HTTP {}): {}", status, text));
        }

        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("JSON decode error ({}): {}", e, text))
    }

    /// Poll for pending commands.
    pub async fn poll_commands(&self) -> Result<CommandsResponse, String> {
        let url = format!(
            "{}/api/desktop/commands/pending?machine_id={}",
            self.base_url, self.machine_id
        );

        let resp = self
            .client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| format!("Command poll network error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read command poll response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Command poll rejected (HTTP {}): {}", status, text));
        }

        match serde_json::from_str::<CommandsResponse>(&text) {
            Ok(parsed) => {
                if parsed.success {
                    Ok(parsed)
                } else {
                    Err(parsed.message.unwrap_or_else(|| "Server rejected command poll".to_string()))
                }
            }
            Err(e) => Err(format!("JSON decode error ({}): {}", e, text)),
        }
    }

    /// Acknowledge a command.
    pub async fn ack_command(
        &self,
        command_id: u64,
        success: bool,
        result: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/desktop/commands/{}/ack",
            self.base_url, command_id
        );

        let body = CommandAckRequest {
            machine_id: self.machine_id.clone(),
            success,
            result,
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ack error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read ack response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Ack rejected (HTTP {}): {}", status, text));
        }

        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("JSON decode error ({}): {}", e, text))
    }

    /// Upload batch connection logs to server.
    pub async fn upload_logs(
        &self,
        logs: Vec<crate::proxy_server::ProxyLogEntry>,
    ) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/desktop/proxy/log", self.base_url);
        let body = serde_json::json!({
            "machine_id": self.machine_id,
            "logs": logs,
        });

        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Log upload error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read log response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Log upload failed (HTTP {}): {}", status, text));
        }

        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("JSON decode error ({}): {}", e, text))
    }

    /// Fetch latest dynamic Rathole client configuration from Laravel server.
    pub async fn get_tunnel_config(&self) -> Result<String, String> {
        let url = format!("{}/api/desktop/tunnel/config?machine_id={}", self.base_url, self.machine_id);

        let resp = self
            .client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| format!("Tunnel config fetch error: {}", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read tunnel config: {}", e))?;

        if !status.is_success() {
            return Err(format!("Tunnel config fetch failed (HTTP {}): {}", status, text));
        }

        Ok(text)
    }
}
