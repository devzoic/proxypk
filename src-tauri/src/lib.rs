mod api;
mod models;
mod proxy_server;
mod updater;

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

#[derive(Clone, Debug)]
struct HardwarePortMeta {
    pub display_name: String,
    pub mac_address: Option<String>,
    pub adapter_type: String,
}

/// Creates a std::process::Command that runs completely silently in the background without opening a CMD or shell window on Windows
#[allow(unused_mut)]
fn create_hidden_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Helper to query hardware ports and MAC addresses across Windows, Linux, and macOS
fn get_hardware_adapters_map() -> HashMap<String, HardwarePortMeta> {
    let mut map = HashMap::new();

    #[cfg(target_os = "windows")]
    {
        // Query PowerShell Get-NetAdapter for exact Name, InterfaceDescription, MacAddress, Status, MediaType (SILENT)
        if let Ok(output) = create_hidden_command("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetAdapter -IncludeHidden | Select-Object -Property Name, InterfaceDescription, MacAddress, Status, MediaType | ConvertTo-Json -Compress"
            ])
            .output()
        {
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let items = if let Some(arr) = val.as_array() {
                        arr.clone()
                    } else if val.is_object() {
                        vec![val]
                    } else {
                        vec![]
                    };

                    for item in items {
                        let name = item["Name"].as_str().unwrap_or("").trim().to_string();
                        let desc = item["InterfaceDescription"].as_str().unwrap_or("").trim().to_string();
                        let raw_mac = item["MacAddress"].as_str().unwrap_or("").trim().to_string();
                        let media = item["MediaType"].as_str().unwrap_or("").to_lowercase();

                        if name.is_empty() {
                            continue;
                        }

                        let mac_address = if !raw_mac.is_empty() && raw_mac != "--" {
                            Some(raw_mac.replace('-', ":").to_uppercase())
                        } else {
                            None
                        };

                        let lower_desc = desc.to_lowercase();
                        let adapter_type = if lower_desc.contains("rndis")
                            || lower_desc.contains("huawei")
                            || lower_desc.contains("zte")
                            || lower_desc.contains("qualcomm")
                            || lower_desc.contains("mobile broadband")
                            || lower_desc.contains("cellular")
                            || lower_desc.contains("modem")
                            || lower_desc.contains("wingle")
                            || media.contains("cellular")
                            || media.contains("wwan")
                        {
                            "wingle".to_string()
                        } else if media.contains("802.3") || lower_desc.contains("ethernet") || lower_desc.contains("gigabit") || lower_desc.contains("realtek") || lower_desc.contains("intel") {
                            "ethernet".to_string()
                        } else if media.contains("native 802.11") || lower_desc.contains("wi-fi") || lower_desc.contains("wireless") || lower_desc.contains("802.11") {
                            "wifi".to_string()
                        } else {
                            "other".to_string()
                        };

                        let display_name = if !desc.is_empty() {
                            format!("{} ({})", name, desc)
                        } else {
                            name.clone()
                        };

                        map.insert(name, HardwarePortMeta {
                            display_name,
                            mac_address,
                            adapter_type,
                        });
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("lo") {
                    continue;
                }

                let mac_path = format!("/sys/class/net/{}/address", name);
                let mac_address = std::fs::read_to_string(&mac_path)
                    .ok()
                    .map(|s| s.trim().to_uppercase())
                    .filter(|s| !s.is_empty() && s != "00:00:00:00:00:00");

                let adapter_type = if name.starts_with("wwan") || name.starts_with("rmnet") || name.starts_with("usb") || name.starts_with("cdc") {
                    "wingle".to_string()
                } else if name.starts_with("wl") {
                    "wifi".to_string()
                } else {
                    "ethernet".to_string()
                };

                map.insert(name.clone(), HardwarePortMeta {
                    display_name: format!("Interface ({})", name),
                    mac_address,
                    adapter_type,
                });
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
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
                            } else if lower_port.contains("huawei") || lower_port.contains("wingle") || lower_port.contains("modem") || lower_port.contains("zte") {
                                "wingle"
                            } else if lower_port.contains("cellular") || lower_port.contains("mobile") || lower_port.contains("lte") {
                                "cellular"
                            } else {
                                "ethernet"
                            };

                            map.insert(
                                current_device.clone(),
                                HardwarePortMeta {
                                    display_name: format!("{} ({})", current_port, current_device),
                                    mac_address: Some(current_mac.to_uppercase()),
                                    adapter_type: adapter_type.to_string(),
                                },
                            );
                        }
                        current_port.clear();
                        current_device.clear();
                    }
                }
            }
        }
    }

    map
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
    let hardware_map = get_hardware_adapters_map();
    let mut interface_ips: HashMap<String, Vec<std::net::IpAddr>> = HashMap::new();

    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in interfaces {
            interface_ips.entry(name).or_default().push(ip);
        }
    }

    let mut adapters = vec![];

    for (name, ips) in interface_ips {
        // Filter out loopback
        if name.starts_with("lo")
            || name.contains("Loopback")
            || name == "127.0.0.1"
            || name == "::1"
        {
            continue;
        }

        let is_virtual = name.starts_with("utun")
            || name.starts_with("awdl")
            || name.starts_with("llw")
            || name.starts_with("bridge")
            || name.starts_with("gif")
            || name.starts_with("stf")
            || name.starts_with("vboxnet")
            || name.starts_with("vEthernet")
            || name.starts_with("docker");

        // Skip virtual interfaces unless requested
        if is_virtual && !allow_virtual {
            continue;
        }

        // Pick primary IPv4 address, fallback to first IPv6
        let primary_v4 = ips.iter().find(|ip| ip.is_ipv4()).map(|ip| ip.to_string());
        let primary_ip = primary_v4.clone().or_else(|| ips.first().map(|ip| ip.to_string()));

        // Filter out disconnected link-local / APIPA (169.254.x.x) or fe80 when virtuals hidden
        if !allow_virtual {
            if let Some(ref ip) = primary_ip {
                if ip.starts_with("169.254.") || ip.starts_with("fe80:") {
                    continue;
                }
            } else {
                continue;
            }
        }

        let (display_name, mac, adapter_type) = if let Some(meta) = hardware_map.get(&name) {
            (
                Some(meta.display_name.clone()),
                meta.mac_address.clone(),
                meta.adapter_type.clone(),
            )
        } else {
            let atype = if name.starts_with("en") || name.starts_with("eth") || name.contains("Ethernet") {
                "ethernet"
            } else if name.starts_with("wl") || name.contains("Wi-Fi") || name.contains("wi-fi") || name.contains("Wireless") {
                "wifi"
            } else if name.starts_with("wwan") || name.starts_with("rmnet") || name.contains("Cellular") || name.contains("Wingle") {
                "wingle"
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

        let gateway = primary_ip.as_ref().and_then(|ip_str| {
            if let Ok(ip) = ip_str.parse::<std::net::Ipv4Addr>() {
                let oct = ip.octets();
                Some(format!("{}.{}.{}.1", oct[0], oct[1], oct[2]))
            } else {
                None
            }
        });

        adapters.push(AdapterInfo {
            adapter_name: name,
            display_name,
            adapter_type,
            mac_address: mac,
            local_ip: primary_ip,
            external_ip: None,
            gateway,
            signal_strength: None,
            connection_speed_mbps: None,
            has_internet: None,
            ping_ms: None,
            is_virtual: Some(is_virtual),
            has_conflict: Some(false),
            conflict_message: None,
            status: "online".to_string(),
        });
    }

    // Detect IP conflicts across adapters (e.g. multiple 4G Wingles with identical 192.168.8.100)
    let mut ip_counts: HashMap<String, usize> = HashMap::new();
    for a in &adapters {
        if let Some(ref ip) = a.local_ip {
            if !ip.is_empty() && !ip.starts_with("127.") && !ip.starts_with("fe80:") {
                *ip_counts.entry(ip.clone()).or_insert(0) += 1;
            }
        }
    }

    for a in &mut adapters {
        if let Some(ref ip) = a.local_ip {
            if let Some(&count) = ip_counts.get(ip) {
                if count > 1 {
                    a.has_conflict = Some(true);
                    a.conflict_message = Some(format!(
                        "IP Conflict: {} other device(s) share IP {}. Multi-wingle setups require distinct subnets (e.g. 192.168.9.1, 192.168.10.1).",
                        count - 1, ip
                    ));
                }
            }
        }
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

/// Sync detected adapters to server with strict filtering (no loopback, no 169.254 APIPA).
#[tauri::command]
async fn sync_adapters(
    adapters: Option<Vec<AdapterInfo>>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let raw_list = adapters.unwrap_or_else(|| detect_adapters(Some(false)));

    let clean_list: Vec<AdapterInfo> = raw_list
        .into_iter()
        .filter(|a| {
            if let Some(ref ip) = a.local_ip {
                !ip.starts_with("169.254.")
                    && !ip.starts_with("127.")
                    && !ip.starts_with("fe80:")
                    && !a.adapter_name.contains("Loopback")
                    && !a.adapter_name.contains("vEthernet")
            } else {
                false
            }
        })
        .collect();

    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Not connected".to_string())?
    };

    client.sync_adapters(clean_list).await?;
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
    authorized_users: Option<Vec<models::AuthorizedUser>>,
    adapter_ip: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mut auth_list = authorized_users.unwrap_or_default();
    if auth_list.is_empty() {
        if let (Some(u), Some(p)) = (username, password) {
            if !u.trim().is_empty() {
                auth_list.push(models::AuthorizedUser {
                    username: u,
                    password: p,
                    user_id: None,
                    subscription_id: None,
                });
            }
        }
    }

    let proto = protocol.unwrap_or_else(|| "socks5".to_string());
    let bind_ip = adapter_ip.and_then(|ip| ip.parse::<std::net::IpAddr>().ok());

    // If already running on the same port and IP, update credentials dynamically in RAM without restart
    {
        let guard = state.running_proxies.lock().unwrap();
        if let Some(instance) = guard.get(&proxy_id) {
            if instance.port == port && instance.bind_adapter_ip == bind_ip {
                instance.update_authorized_users(auth_list);
                log::info!("Updated in-memory credentials for proxy server #{} on port {}", proxy_id, port);
                return Ok(format!("Proxy #{} credentials updated on port {}", proxy_id, port));
            }
        }
    }

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

    let instance = proxy_server::ProxyInstance::start(
        proxy_id,
        port,
        proto.clone(),
        auth_list,
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

/// Helper to obtain a reliable, 100% user-writable runtime directory across Windows, Linux, and macOS.
fn get_runtime_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);

    let dir = base.join("proxypk");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Automatically ensure the Rathole binary exists locally for the current OS/architecture, downloading if needed.
async fn ensure_rathole_binary() -> Result<std::path::PathBuf, String> {
    let runtime_dir = get_runtime_dir();
    let exe_name = if cfg!(target_os = "windows") { "rathole.exe" } else { "rathole" };
    let target_path = runtime_dir.join(exe_name);

    if target_path.exists() {
        return Ok(target_path);
    }

    // Check system PATH
    if let Ok(output) = create_hidden_command(exe_name).arg("--version").output() {
        if output.status.success() {
            return Ok(std::path::PathBuf::from(exe_name));
        }
    }

    // Also check next to executable
    if let Ok(app_exe) = std::env::current_exe() {
        if let Some(parent) = app_exe.parent() {
            let next_to_exe = parent.join(exe_name);
            if next_to_exe.exists() {
                return Ok(next_to_exe);
            }
        }
    }

    log::info!("Rathole binary missing. Automatically downloading to {:?}...", target_path);

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

    let mut toml_text = client.get_tunnel_config().await?;

    // Robust hostname resolution: If server returned 127.0.0.1 or localhost, fallback to relay.devzoic.com
    if toml_text.contains("127.0.0.1:2333") || toml_text.contains("localhost:2333") {
        toml_text = toml_text.replace("127.0.0.1:2333", "relay.devzoic.com:2333")
            .replace("localhost:2333", "relay.devzoic.com:2333");
    }

    // Determine user-writable config path in runtime directory
    let runtime_dir = get_runtime_dir();
    let config_path = runtime_dir.join("client.toml");

    std::fs::write(&config_path, &toml_text)
        .map_err(|e| format!("Failed to write client.toml at {:?}: {}", config_path, e))?;

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
        // Terminate any detached stale rathole instances
        #[cfg(target_os = "windows")]
        {
            let _ = create_hidden_command("taskkill")
                .args(["/F", "/IM", "rathole.exe"])
                .output();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = create_hidden_command("killall")
                .args(["-9", "rathole"])
                .output();
        }

        let config_str = config_path.to_string_lossy().to_string();
        match create_hidden_command(&exe_path)
            .args(["--client", &config_str])
            .spawn()
        {
            Ok(child) => {
                log::info!("Spawned Rathole reverse tunnel (PID: {}) using config {:?}", child.id(), config_path);
                *child_guard = Some(child);
                is_running = true;
            }
            Err(e) => {
                return Err(format!("Failed to launch Rathole executable {:?}: {}", exe_path, e));
            }
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

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct FetchProxiesResult {
    pub success: bool,
    pub total_proxies: usize,
    pub running_started: usize,
    pub tunnel_status: Option<TunnelStatus>,
    pub message: String,
}

/// Fetch proxy configuration from Laravel control plane and start all running proxy listeners + reverse tunnel
#[tauri::command]
async fn fetch_and_start_proxies(state: State<'_, AppState>) -> Result<FetchProxiesResult, String> {
    let client = {
        let guard = state.api_client.lock().unwrap();
        guard.clone().ok_or_else(|| "Agent not connected to server".to_string())?
    };

    let data = client.get_proxy_configs().await?;
    let proxies = data["proxies"].as_array().cloned().unwrap_or_default();
    let total_count = proxies.len();
    let mut started_count = 0;
    let mut active_proxy_ids = std::collections::HashSet::new();

    for p in &proxies {
        let status = p["status"].as_str().unwrap_or("");
        if status == "running" {
            let proxy_id = p["id"].as_u64().unwrap_or(0);
            let port = p["local_port"].as_u64().unwrap_or(0) as u16;
            let protocol = p["protocol"].as_str().map(|s| s.to_string());
            let adapter_ip = p["network_adapter"]["local_ip"].as_str().map(|s| s.to_string());

            let mut auth_users: Vec<models::AuthorizedUser> = Vec::new();
            if let Some(arr) = p["authorized_users"].as_array() {
                for u_val in arr {
                    if let (Some(u), Some(pw)) = (u_val["username"].as_str(), u_val["password"].as_str()) {
                        auth_users.push(models::AuthorizedUser {
                            username: u.to_string(),
                            password: pw.to_string(),
                            user_id: u_val["user_id"].as_u64(),
                            subscription_id: u_val["subscription_id"].as_u64(),
                        });
                    }
                }
            }

            if auth_users.is_empty() {
                if let (Some(u), Some(pw)) = (p["username"].as_str(), p["password"].as_str()) {
                    if !u.trim().is_empty() {
                        auth_users.push(models::AuthorizedUser {
                            username: u.to_string(),
                            password: pw.to_string(),
                            user_id: None,
                            subscription_id: None,
                        });
                    }
                }
            }

            if port > 0 {
                active_proxy_ids.insert(proxy_id);
                let bind_ip = adapter_ip.and_then(|ip| ip.parse::<std::net::IpAddr>().ok());
                let proto = protocol.unwrap_or_else(|| "both".to_string());

                // Check if already running on same port and adapter
                let already_running = {
                    let guard = state.running_proxies.lock().unwrap();
                    if let Some(instance) = guard.get(&proxy_id) {
                        if instance.port == port && instance.bind_adapter_ip == bind_ip {
                            instance.update_authorized_users(auth_users.clone());
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if already_running {
                    started_count += 1;
                    continue;
                }

                // Stop duplicate / old instance if running
                {
                    let mut guard = state.running_proxies.lock().unwrap();
                    if let Some(existing) = guard.remove(&proxy_id) {
                        existing.stop();
                    }
                }

                if let Ok(instance) = proxy_server::ProxyInstance::start(
                    proxy_id,
                    port,
                    proto,
                    auth_users,
                    bind_ip,
                ).await {
                    let mut guard = state.running_proxies.lock().unwrap();
                    guard.insert(proxy_id, instance);
                    started_count += 1;
                }
            }
        }
    }

    // Stop any proxies that were deleted or marked stopped in Laravel
    {
        let mut guard = state.running_proxies.lock().unwrap();
        let to_remove: Vec<u64> = guard.keys()
            .copied()
            .filter(|id| !active_proxy_ids.contains(id))
            .collect();

        for id in to_remove {
            if let Some(instance) = guard.remove(&id) {
                instance.stop();
                log::info!("Stopped decommissioned proxy #{}", id);
            }
        }
    }

    // Now synchronize and supervise the reverse tunnel
    let tunnel_status = sync_and_start_tunnel(state).await.ok();

    Ok(FetchProxiesResult {
        success: true,
        total_proxies: total_count,
        running_started: started_count,
        tunnel_status,
        message: format!("Synchronized {} proxy listener(s) from dashboard", started_count),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WingleSubnetConfigResult {
    pub success: bool,
    pub message: String,
    pub old_ip: String,
    pub new_ip: String,
}

/// Automatically configure Huawei / Zong / ZTE HiLink 4G Wingle subnet LAN IP (e.g. 192.168.8.1 -> 192.168.9.1)
#[tauri::command]
async fn configure_wingle_subnet(
    current_gateway_ip: String,
    new_gateway_ip: String,
) -> Result<WingleSubnetConfigResult, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let base_url = format!("http://{}", current_gateway_ip.trim());

    // 1. Fetch Session & Token info from HiLink dongle
    let token_url = format!("{}/api/webserver/SesTokInfo", base_url);
    let token_resp = client.get(&token_url).send().await;

    let mut session_id = String::new();
    let mut token = String::new();

    if let Ok(resp) = token_resp {
        if let Some(tok_header) = resp.headers().get("__RequestVerificationToken") {
            if let Ok(h_str) = tok_header.to_str() {
                token = h_str.to_string();
            }
        }
        if let Some(cookie_header) = resp.headers().get("Set-Cookie") {
            if let Ok(c_str) = cookie_header.to_str() {
                if let Some(pos) = c_str.find("SessionID=") {
                    let rest = &c_str[pos + 10..];
                    let end = rest.find(';').unwrap_or(rest.len());
                    session_id = rest[..end].to_string();
                }
            }
        }
        if let Ok(text) = resp.text().await {
            if token.is_empty() {
                if let Some(pos) = text.find("<TokInfo>") {
                    if let Some(end) = text[pos + 9..].find("</TokInfo>") {
                        token = text[pos + 9..pos + 9 + end].to_string();
                    }
                }
            }
            if session_id.is_empty() {
                if let Some(pos) = text.find("<SesInfo>") {
                    if let Some(end) = text[pos + 9..].find("</SesInfo>") {
                        session_id = text[pos + 9..pos + 9 + end].to_string();
                    }
                }
            }
        }
    }

    // Determine new DHCP range (e.g. if 192.168.9.1 -> 192.168.9.100 - 192.168.9.200)
    let parts: Vec<&str> = new_gateway_ip.split('.').collect();
    if parts.len() != 4 {
        return Err("Invalid new gateway IP format. Expected format like 192.168.9.1".to_string());
    }
    let prefix = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
    let start_ip = format!("{}.100", prefix);
    let end_ip = format!("{}.200", prefix);

    let xml_payload = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><request><DhcpIPAddress>{}</DhcpIPAddress><DhcpLanNetmask>255.255.255.0</DhcpLanNetmask><DhcpStatus>1</DhcpStatus><DhcpStartIPAddress>{}</DhcpStartIPAddress><DhcpEndIPAddress>{}</DhcpEndIPAddress><DhcpLeaseTime>86400</DhcpLeaseTime><DnsStatus>1</DnsStatus><PrimaryDns>{}</PrimaryDns><SecondaryDns>{}</SecondaryDns></request>",
        new_gateway_ip.trim(),
        start_ip,
        end_ip,
        new_gateway_ip.trim(),
        new_gateway_ip.trim(),
    );

    let dhcp_url = format!("{}/api/dhcp/settings", base_url);
    let mut req = client.post(&dhcp_url)
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8");

    if !token.is_empty() {
        req = req.header("__RequestVerificationToken", &token);
    }
    if !session_id.is_empty() {
        req = req.header("Cookie", format!("SessionID={}", session_id));
    }

    let hilink_res = req.body(xml_payload).send().await;

    if let Ok(resp) = hilink_res {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if text.contains("<response>OK</response>") || text.contains("<response>1</response>") || (status.is_success() && text.contains("OK")) {
            return Ok(WingleSubnetConfigResult {
                success: true,
                message: format!(
                    "Successfully updated Wingle LAN IP to {}. The dongle is rebooting its DHCP server.",
                    new_gateway_ip
                ),
                old_ip: current_gateway_ip,
                new_ip: new_gateway_ip,
            });
        }
    }

    // 2. Fallback: ZTE / Qualcomm modem endpoint
    let zte_url = format!("{}/goform/goform_set_cmd_process", base_url);
    let zte_params = [
        ("isTest", "false"),
        ("goformId", "SET_LAN_RULE"),
        ("lan_ipaddr", new_gateway_ip.trim()),
        ("lan_netmask", "255.255.255.0"),
        ("dhcp_start", &start_ip),
        ("dhcp_end", &end_ip),
        ("dhcp_lease_time", "86400"),
    ];

    if let Ok(resp) = client.post(&zte_url).form(&zte_params).send().await {
        let text = resp.text().await.unwrap_or_default();
        if text.contains("success") || text.contains("OK") {
            return Ok(WingleSubnetConfigResult {
                success: true,
                message: format!(
                    "Successfully updated ZTE Wingle LAN IP to {}. The dongle is rebooting its DHCP server.",
                    new_gateway_ip
                ),
                old_ip: current_gateway_ip,
                new_ip: new_gateway_ip,
            });
        }
    }

    Err(format!(
        "Dongle requires manual web admin authentication. Click 'Open Web Admin' below (http://{}) to set the LAN IP to {}.",
        current_gateway_ip, new_gateway_ip
    ))
}

/// Open Wingle Web Admin portal in default browser
#[tauri::command]
async fn open_wingle_portal(gateway_ip: String, app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let url = if gateway_ip.starts_with("http://") || gateway_ip.starts_with("https://") {
        gateway_ip
    } else {
        format!("http://{}/html/dhcpipaddress.html", gateway_ip.trim())
    };
    app.opener().open_url(&url, None::<&str>).map_err(|e| e.to_string())?;
    Ok(format!("Opened {}", url))
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
            updater::download_and_install_update,
            sync_and_start_tunnel,
            restart_tunnel,
            get_running_proxies,
            fetch_and_start_proxies,
            configure_wingle_subnet,
            open_wingle_portal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
