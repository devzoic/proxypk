use serde::{Deserialize, Serialize};

/// Server configuration
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub api_url: String,
    pub token: Option<String>,
    pub machine_id: String,
}

/// Device registration request
#[derive(Debug, Serialize)]
pub struct RegisterRequest {
    pub machine_id: String,
    pub hostname: String,
    pub os_type: String,
    pub os_version: String,
    pub agent_version: String,
}

/// Device registration response
#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub success: bool,
    pub device_id: Option<u64>,
    pub token: Option<String>,
    pub is_approved: Option<bool>,
    pub message: Option<String>,
}

/// Heartbeat request
#[derive(Debug, Serialize)]
pub struct HeartbeatRequest {
    pub machine_id: String,
    pub public_ip: Option<String>,
    pub agent_version: String,
    pub cpu_usage: Option<f64>,
    pub ram_usage: Option<f64>,
    pub active_connections: Option<u32>,
}

/// Heartbeat response
#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    pub success: bool,
    pub is_approved: Option<bool>,
    pub pending_commands: Option<u32>,
}

/// Network adapter info
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AdapterInfo {
    pub adapter_name: String,
    pub display_name: Option<String>,
    pub adapter_type: String,
    pub mac_address: Option<String>,
    pub local_ip: Option<String>,
    pub external_ip: Option<String>,
    pub gateway: Option<String>,
    pub signal_strength: Option<i32>,
    pub connection_speed_mbps: Option<f64>,
    pub has_internet: Option<bool>,
    pub ping_ms: Option<u64>,
    pub is_virtual: Option<bool>,
    pub has_conflict: Option<bool>,
    pub conflict_message: Option<String>,
    pub status: String,
}

/// Internet check result for an adapter
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InternetCheckResult {
    pub has_internet: bool,
    pub external_ip: Option<String>,
    pub ping_ms: Option<u64>,
    pub error: Option<String>,
}

/// Adapter sync request
#[derive(Debug, Serialize)]
pub struct AdapterSyncRequest {
    pub machine_id: String,
    pub adapters: Vec<AdapterInfo>,
}

/// Command from server
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeviceCommand {
    pub id: u64,
    pub command: String,
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub created_at: Option<String>,
}

/// Command poll response
#[derive(Debug, Deserialize)]
pub struct CommandsResponse {
    pub success: bool,
    #[serde(default)]
    pub commands: Vec<DeviceCommand>,
    pub message: Option<String>,
}

/// Command acknowledge request
#[derive(Debug, Serialize)]
pub struct CommandAckRequest {
    pub machine_id: String,
    pub success: bool,
    pub result: Option<serde_json::Value>,
}

/// Agent state exposed to frontend
#[derive(Debug, Serialize, Clone)]
pub struct AgentState {
    pub connected: bool,
    pub registered: bool,
    pub approved: bool,
    pub machine_id: String,
    pub hostname: String,
    pub os_type: String,
    pub status: String,
    pub adapters: Vec<AdapterInfo>,
    pub pending_commands: u32,
    pub last_heartbeat: Option<String>,
    pub error: Option<String>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            connected: false,
            registered: false,
            approved: false,
            machine_id: String::new(),
            hostname: String::new(),
            os_type: String::new(),
            status: "offline".to_string(),
            adapters: vec![],
            pending_commands: 0,
            last_heartbeat: None,
            error: None,
        }
    }
}

/// Authorized proxy credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedUser {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub subscription_id: Option<u64>,
}

