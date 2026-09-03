mod api;
mod models;
mod proxy_server;

use api::ApiClient;
use models::{AdapterInfo, AgentState, InternetCheckResult};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// Shared application state
pub struct AppState {
    pub agent: Mutex<AgentState>,
    pub api_client: Mutex<Option<ApiClient>>,
    pub running_proxies: Mutex<HashMap<u64, proxy_server::ProxyInstance>>,
}

/// Helper on macOS to query real hardware ports and map devices to friendly names
#[cfg(target_os = "macos")]
fn get_macos_hardware_ports() -> HashMap<String, (String, String, String)> {
    let mut map = HashMap::new();

    if let Ok(output) = std::process::Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut current_port = String::new();
            let mut current_device = String::new();

            for line in stdout.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Hardware Port:") {
                    current_port = trimmed.trim_start_matches("Hardware Port:").trim().to_string();
                } else if trimmed.starts_with("Device:") {
                    current_device = trimmed.trim_start_matches("Device:").trim().to_string();
                } else if trimmed.starts_with("Ethernet Address:") {
                    let current_mac = trimmed.trim_start_matches("Ethernet Address:").trim().to_string();

                    if !current_device.is_empty() {
                        let lower_port = current_port.to_lowercase();
                        let adapter_type = if lower_port.contains("wi-fi") || lower_port.contains("wifi") || lower_port.contains("wireless") {
                            "wifi"
                        } else if lower_port.contains("ethernet") || lower_port.contains("lan") || lower_port.contains("thunderbolt") {
                            "ethernet"
                        } else if lower_port.contains("huawei") || lower_port.contains("wingle") || lower_port.contains("modem") || lower_port.contains("zte") {
                            "wingle"
                        } else if lower_port.contains("cellular") || lower_port.contains("mobile") || lower_port.contains("lte") {
                            "cellular"
                        } else {
                            "ethernet"
                        };

                        map.insert(
                            current_device.clone(),
                            (current_port.clone(), current_mac, adapter_type.to_string()),
                        );
                    }
                    current_port.clear();
                    current_device.clear();
                }
            }
        }
    }

    map
}

#[cfg(not(target_os = "macos"))]
fn get_macos_hardware_ports() -> HashMap<String, (String, String, String)> {
    HashMap::new()
}

/// Initialize the agent by connecting to the server.
#[tauri::command]
async fn connect_to_server(
    server_url: String,
    state: State<'_, AppState>,
) -> Result<AgentState, String> {
    let machine_id = machine_uid::get().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let os_type = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    };

    let os_version = std::env::consts::OS.to_string();

    // Create API client
    let mut client = ApiClient::new(&server_url, &machine_id);

    // Try to register
    let reg = client.register(&hostname, os_type, &os_version).await?;

    if let Some(token) = reg.token {
        client.set_token(token);
    }

    let result = {
        let mut agent = state.agent.lock().unwrap();
        agent.machine_id = machine_id;
        agent.hostname = hostname;
        agent.os_type = os_type.to_string();
        agent.registered = reg.success;
        agent.approved = reg.is_approved.unwrap_or(false);
        agent.connected = true;
        agent.status = if reg.is_approved.unwrap_or(false) {
            "online".to_string()
        } else if reg.success {
            "registered".to_string()
        } else {
            "pending".to_string()
        };
        agent.clone()
    };

    // Store the client
    *state.api_client.lock().unwrap() = Some(client);

    Ok(result)
}

/// Send heartbeat to server.
#[tauri::command]
async fn send_heartbeat(state: State<'_, AppState>) -> Result<AgentState, String> {
    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    let hb = client.heartbeat().await?;

    let agent = {
        let mut agent = state.agent.lock().unwrap();
        agent.approved = hb.is_approved.unwrap_or(false);
        agent.pending_commands = hb.pending_commands.unwrap_or(0);
        agent.last_heartbeat = Some(chrono::Utc::now().to_rfc3339());
        agent.status = "online".to_string();
        agent.clone()
    };

    Ok(agent)
}

/// Get current agent state.
#[tauri::command]
fn get_agent_state(state: State<'_, AppState>) -> AgentState {
    state.agent.lock().unwrap().clone()
}

/// Get network adapters detected on this machine with clean grouping and hardware info.
#[tauri::command]
fn detect_adapters(include_virtual: Option<bool>) -> Vec<AdapterInfo> {
    let allow_virtual = include_virtual.unwrap_or(false);
    let hardware_map = get_macos_hardware_ports();
    let mut interface_ips: HashMap<String, Vec<std::net::IpAddr>> = HashMap::new();

    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in interfaces {
            interface_ips.entry(name).or_default().push(ip);
        }
    }

    let mut adapters = vec![];

    for (name, ips) in interface_ips {
        // Filter out loopback
        if name.starts_with("lo") || name == "127.0.0.1" || name == "::1" {
            continue;
        }

        let is_virtual = name.starts_with("utun")
            || name.starts_with("awdl")
            || name.starts_with("llw")
            || name.starts_with("bridge")
            || name.starts_with("gif")
            || name.starts_with("stf")
            || name.starts_with("vboxnet")
            || name.starts_with("docker");

        // Skip virtual interfaces unless requested
        if is_virtual && !allow_virtual {
            continue;
        }

        // Pick primary IPv4 address, fallback to first IPv6
        let primary_v4 = ips.iter().find(|ip| ip.is_ipv4()).map(|ip| ip.to_string());
        let primary_ip = primary_v4.clone().or_else(|| ips.first().map(|ip| ip.to_string()));

        // Don't show interfaces with only fe80 link-local addresses when virtuals are hidden
        if !allow_virtual && primary_ip.as_ref().map_or(true, |ip| ip.starts_with("fe80:")) {
            continue;
        }

        let (display_name, mac, adapter_type) = if let Some((port_name, mac_addr, atype)) = hardware_map.get(&name) {
            (
                Some(format!("{} ({})", port_name, name)),
                Some(mac_addr.clone()),
                atype.clone(),
            )
        } else {
            let atype = if name.starts_with("en") || name.starts_with("eth") {
                "ethernet"
            } else if name.starts_with("wl") || name.contains("Wi-Fi") || name.contains("wi-fi") {
                "wifi"
            } else if name.starts_with("wwan") || name.starts_with("rmnet") {
                "cellular"
            } else if is_virtual {
                "vpn"
            } else {
                "other"
            };

            let dname = if is_virtual {
                format!("Virtual Tunnel ({})", name)
            } else {
                format!("Interface ({})", name)
            };

            (Some(dname), None, atype.to_string())
        };

        adapters.push(AdapterInfo {
            adapter_name: name,
            display_name,
            adapter_type,
            mac_address: mac,
            local_ip: primary_ip,
            external_ip: None,
            gateway: None,
            signal_strength: None,
            connection_speed_mbps: None,
            has_internet: None,
            ping_ms: None,
            is_virtual: Some(is_virtual),
            status: "online".to_string(),
        });
    }

    // Sort: real hardware adapters with IPv4 first
    adapters.sort_by(|a, b| {
        let a_virt = a.is_virtual.unwrap_or(false);
        let b_virt = b.is_virtual.unwrap_or(false);
        if a_virt != b_virt {
            return a_virt.cmp(&b_virt);
        }
        let a_has_v4 = a.local_ip.as_ref().map_or(false, |ip| !ip.contains(':'));
        let b_has_v4 = b.local_ip.as_ref().map_or(false, |ip| !ip.contains(':'));
        if a_has_v4 != b_has_v4 {
            return b_has_v4.cmp(&a_has_v4);
        }
        a.adapter_name.cmp(&b.adapter_name)
    });

    adapters
}

/// Test internet connectivity for a specific network interface by binding to its local IP address.
#[tauri::command]
async fn check_adapter_internet(ip: String) -> Result<InternetCheckResult, String> {
    let local_addr = match ip.parse::<std::net::IpAddr>() {
        Ok(addr) => addr,
        Err(e) => {
            return Ok(InternetCheckResult {
                has_internet: false,
                external_ip: None,
                ping_ms: None,
                error: Some(format!("Invalid IP address: {}", e)),
            })
        }
    };

    if local_addr.is_loopback() {
        return Ok(InternetCheckResult {
            has_internet: false,
            external_ip: None,
            ping_ms: None,
            error: Some("Loopback interface has no external route".to_string()),
        });
    }

    let is_link_local = match local_addr {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    };

    if is_link_local {
        return Ok(InternetCheckResult {
            has_internet: false,
            external_ip: None,
            ping_ms: None,
            error: Some("Link-local IPv6 address (local-only)".to_string()),
        });
    }

    let client = match reqwest::Client::builder()
        .local_address(local_addr)
        .timeout(std::time::Duration::from_millis(3500))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(InternetCheckResult {
                has_internet: false,
                external_ip: None,
                ping_ms: None,
                error: Some(format!("Could not bind to local IP: {}", e)),
            })
        }
    };

    let start = std::time::Instant::now();
    let test_endpoints = [
        "https://api.ipify.org",
        "https://checkip.amazonaws.com",
        "https://icanhazip.com",
    ];

    for endpoint in test_endpoints {
        if let Ok(resp) = client.get(endpoint).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.text().await {
                    let ext_ip = body.trim().to_string();
                    if !ext_ip.is_empty() && ext_ip.len() <= 45 {
                        let ping = start.elapsed().as_millis() as u64;
                        return Ok(InternetCheckResult {
                            has_internet: true,
                            external_ip: Some(ext_ip),
                            ping_ms: Some(ping),
                            error: None,
                        });
                    }
                }
            }
        }
    }

    Ok(InternetCheckResult {
        has_internet: false,
        external_ip: None,
        ping_ms: None,
        error: Some("No internet route or connection timed out".to_string()),
    })
}

/// Sync detected adapters to server.
#[tauri::command]
async fn sync_adapters(
    adapters: Option<Vec<AdapterInfo>>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let adapter_list = adapters.unwrap_or_else(|| detect_adapters(Some(false)));

    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    client.sync_adapters(adapter_list).await?;
    Ok("Adapters synced successfully".to_string())
}

/// Poll pending commands.
#[tauri::command]
async fn poll_commands(state: State<'_, AppState>) -> Result<Vec<models::DeviceCommand>, String> {
    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    let resp = client.poll_commands().await?;
    Ok(resp.commands)
}

/// Start a real SOCKS5 / HTTP Proxy server listener on specified port bound to interface IP.
#[tauri::command]
async fn start_proxy_server(
    proxy_id: u64,
    port: u16,
    protocol: Option<String>,
    username: Option<String>,
    password: Option<String>,
    adapter_ip: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // If a proxy with this proxy_id or port is already running in state, stop it first
    {
        let mut guard = state.running_proxies.lock().unwrap();
        if let Some(existing) = guard.remove(&proxy_id) {
            existing.stop();
        }
        // Also check if another proxy_id has the same port
        let mut duplicate_id = None;
        for (id, instance) in guard.iter() {
            if instance.port == port {
                instance.stop();
                duplicate_id = Some(*id);
                break;
            }
        }
        if let Some(dup_id) = duplicate_id {
            guard.remove(&dup_id);
        }
    }

    let proto = protocol.unwrap_or_else(|| "socks5".to_string());
    let bind_ip = adapter_ip.and_then(|ip| ip.parse::<std::net::IpAddr>().ok());

    let instance = proxy_server::ProxyInstance::start(
        proxy_id,
        port,
        proto.clone(),
        username,
        password,
        bind_ip,
    )
    .await?;

    let mut guard = state.running_proxies.lock().unwrap();
    guard.insert(proxy_id, instance);

    log::info!("Started local proxy server #{} on port {}", proxy_id, port);
    Ok(format!("Proxy #{} successfully started on port {}", proxy_id, port))
}

/// Stop a running proxy listener.
#[tauri::command]
fn stop_proxy_server(proxy_id: u64, state: State<'_, AppState>) -> Result<String, String> {
    let mut guard = state.running_proxies.lock().unwrap();
    if let Some(instance) = guard.remove(&proxy_id) {
        instance.stop();
        log::info!("Stopped proxy server #{}", proxy_id);
        Ok(format!("Proxy #{} stopped", proxy_id))
    } else {
        Ok(format!("Proxy #{} was not active", proxy_id))
    }
}

/// Acknowledge command execution status to server.
#[tauri::command]
async fn ack_command(
    command_id: u64,
    success: bool,
    result: Option<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    client.ack_command(command_id, success, result).await
}

/// Flush all pending in-memory proxy connection logs to the central Laravel server.
#[tauri::command]
async fn flush_proxy_logs(state: State<'_, AppState>) -> Result<usize, String> {
    let logs = proxy_server::drain_logs();
    if logs.is_empty() {
        return Ok(0);
    }
    let count = logs.len();

    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    client.upload_logs(logs).await?;
    Ok(count)
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    available: bool,
    version: Option<String>,
    current_version: String,
    body: Option<String>,
    date: Option<String>,
    download_url: Option<String>,
}

#[tauri::command]
async fn get_app_version(app: tauri::AppHandle) -> Result<String, String> {
    Ok(app.package_info().version.to_string())
}

#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent("ProxyPK-Desktop-Agent")
        .build()
        .map_err(|e| e.to_string())?;

    match client
        .get("https://api.github.com/repos/devzoic/proxypk/releases/latest")
        .send()
        .await
    {
        Ok(res) => {
            if res.status().is_success() {
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    let tag = json["tag_name"]
                        .as_str()
                        .unwrap_or("")
                        .trim_start_matches('v');
                    let body = json["body"].as_str().map(|s| s.to_string());
                    let date = json["published_at"].as_str().map(|s| s.to_string());
                    let html_url = json["html_url"].as_str().map(|s| s.to_string());

                    let is_newer = !tag.is_empty() && tag != current_version;

                    return Ok(UpdateInfo {
                        available: is_newer,
                        version: if is_newer {
                            Some(tag.to_string())
                        } else {
                            None
                        },
                        current_version,
                        body,
                        date,
                        download_url: html_url,
                    });
                }
            }
        }
        Err(e) => {
            log::warn!("Failed to check updates: {}", e);
        }
    }

    Ok(UpdateInfo {
        available: false,
        version: None,
        current_version,
        body: None,
        date: None,
        download_url: None,
    })
}

#[tauri::command]
async fn install_update() -> Result<String, String> {
    let url = "https://github.com/devzoic/proxypk/releases/latest";
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok("Opening latest download release page in browser...".to_string())
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct TunnelStatus {
    pub active: bool,
    pub running_services_count: usize,
    pub config_synced: bool,
    pub message: String,
}

static TUNNEL_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// Automatically ensure the Rathole binary exists locally for the current OS/architecture, downloading if needed.
async fn ensure_rathole_binary() -> Result<std::path::PathBuf, String> {
    let app_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir());

    let exe_name = if cfg!(target_os = "windows") { "rathole.exe" } else { "rathole" };
    let target_path = app_dir.join(exe_name);

    if target_path.exists() {
        return Ok(target_path);
    }

    // Check system PATH
    if let Ok(output) = std::process::Command::new(exe_name).arg("--version").output() {
        if output.status.success() {
            return Ok(std::path::PathBuf::from(exe_name));
        }
    }

    log::info!("Rathole binary missing. Automatically downloading for current OS/Arch...");

    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let download_url = match (os, arch) {
        ("windows", "x86_64") => "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-pc-windows-msvc.zip",
        ("linux", "x86_64") => "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-unknown-linux-gnu.zip",
        ("linux", "aarch64") => "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-aarch64-unknown-linux-gnu.zip",
        ("macos", "x86_64") => "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-apple-darwin.zip",
        ("macos", "aarch64") => "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-aarch64-apple-darwin.zip",
        _ => return Err(format!("Unsupported OS ({}) and Architecture ({}) for automated Rathole download", os, arch)),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to create download client: {}", e))?;

    let bytes = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download Rathole binary from {}: {}", download_url, e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read downloaded binary bytes: {}", e))?;

    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Failed to parse downloaded zip archive: {}", e))?;

    let mut extracted = false;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Zip archive index error: {}", e))?;
        let name = file.name().to_string();
        if name == "rathole" || name == "rathole.exe" || name.ends_with("/rathole") || name.ends_with("/rathole.exe") {
            let mut out = std::fs::File::create(&target_path)
                .map_err(|e| format!("Failed to create destination file {:?}: {}", target_path, e))?;
            std::io::copy(&mut file, &mut out)
                .map_err(|e| format!("Failed to extract file contents: {}", e))?;
            extracted = true;
            break;
        }
    }

    if !extracted {
        return Err("Rathole executable not found inside downloaded zip package".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&target_path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&target_path, perms);
        }
    }

    log::info!("Successfully downloaded and provisioned Rathole at {:?}", target_path);
    Ok(target_path)
}

/// Fetch latest tunnel config from server, write client.toml, and supervise background Rathole process
#[tauri::command]
async fn sync_and_start_tunnel(state: State<'_, AppState>) -> Result<TunnelStatus, String> {
    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Agent not connected to server".to_string())?
    };

    let toml_text = client.get_tunnel_config().await?;

    // Determine config path in current directory or temp
    let config_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("client.toml")))
        .unwrap_or_else(|| std::path::PathBuf::from("client.toml"));

    let _ = std::fs::write(&config_path, &toml_text);

    let exe_path = match ensure_rathole_binary().await {
        Ok(path) => path,
        Err(e) => {
            log::warn!("Could not auto-provision Rathole binary: {}", e);
            let rathole_exe = if cfg!(target_os = "windows") { "rathole.exe" } else { "rathole" };
            std::path::PathBuf::from(rathole_exe)
        }
    };

    let mut is_running = false;
    let mut child_guard = TUNNEL_CHILD.lock().unwrap();

    if let Some(child) = child_guard.as_mut() {
        if let Ok(None) = child.try_wait() {
            is_running = true;
        }
    }

    if !is_running {
        let config_str = config_path.to_str().unwrap_or("client.toml");
        if let Ok(child) = std::process::Command::new(&exe_path)
            .args(["--client", config_str])
            .spawn()
        {
            *child_guard = Some(child);
            is_running = true;
        }
    }

    let running_proxies_count = state.running_proxies.lock().unwrap().len();

    Ok(TunnelStatus {
        active: is_running,
        running_services_count: running_proxies_count,
        config_synced: true,
        message: if is_running {
            "Rathole reverse tunnel is actively routing in the background".to_string()
        } else {
            "Tunnel config updated (client.toml written)".to_string()
        },
    })
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct RunningProxyInfo {
    pub proxy_id: u64,
    pub port: u16,
    pub protocol: String,
    pub bind_adapter_ip: Option<String>,
    pub active_connections: u64,
    pub bytes_transferred: u64,
    pub is_running: bool,
}

/// Retrieve all active local proxy listener instances and their status
#[tauri::command]
fn get_running_proxies(state: State<'_, AppState>) -> Result<Vec<RunningProxyInfo>, String> {
    let guard = state.running_proxies.lock().unwrap();
    let mut list = Vec::new();
    for (_id, instance) in guard.iter() {
        list.push(RunningProxyInfo {
            proxy_id: instance.proxy_id,
            port: instance.port,
            protocol: instance.protocol.clone(),
            bind_adapter_ip: instance.bind_adapter_ip.map(|ip| ip.to_string()),
            active_connections: instance.active_connections.load(std::sync::atomic::Ordering::Relaxed),
            bytes_transferred: instance.bytes_transferred.load(std::sync::atomic::Ordering::Relaxed),
            is_running: instance.is_running.load(std::sync::atomic::Ordering::Relaxed),
        });
    }
    list.sort_by_key(|p| p.proxy_id);
    Ok(list)
}

/// Force restart the background Rathole tunnel process
#[tauri::command]
async fn restart_tunnel(state: State<'_, AppState>) -> Result<TunnelStatus, String> {
    {
        let mut child_guard = TUNNEL_CHILD.lock().unwrap();
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill();
        }
    }
    sync_and_start_tunnel(state).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            agent: Mutex::new(AgentState::default()),
            api_client: Mutex::new(None),
            running_proxies: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            connect_to_server,
            send_heartbeat,
            get_agent_state,
            detect_adapters,
            check_adapter_internet,
            sync_adapters,
            poll_commands,
            ack_command,
            start_proxy_server,
            stop_proxy_server,
            flush_proxy_logs,
            get_app_version,
            check_for_updates,
            install_update,
            sync_and_start_tunnel,
            restart_tunnel,
            get_running_proxies,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
