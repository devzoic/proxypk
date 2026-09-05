use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::broadcast;

/// Structured connection log entry for high-performance batch shipping
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyLogEntry {
    pub proxy_id: u64,
    pub username: Option<String>,
    pub source_ip: Option<String>,
    pub destination_host: Option<String>,
    pub destination_port: Option<u16>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub duration_ms: u64,
    pub protocol: String,
}

static PENDING_LOGS: Mutex<Vec<ProxyLogEntry>> = Mutex::new(Vec::new());

pub fn record_log(entry: ProxyLogEntry) {
    if let Ok(mut logs) = PENDING_LOGS.lock() {
        if logs.len() < 10000 {
            logs.push(entry);
        }
    }
}

pub fn drain_logs() -> Vec<ProxyLogEntry> {
    if let Ok(mut logs) = PENDING_LOGS.lock() {
        std::mem::take(&mut *logs)
    } else {
        Vec::new()
    }
}

/// In-Memory Thread-Safe High-Performance Asynchronous DNS Cache (60s TTL)
/// Bypasses repetitive DNS queries during multi-stream speedtests and web scraping.
struct DnsCacheEntry {
    addresses: Vec<SocketAddr>,
    expires_at: Instant,
}

static DNS_CACHE: Mutex<Option<HashMap<String, DnsCacheEntry>>> = Mutex::new(None);

async fn resolve_host_cached(target: &str) -> Result<Vec<SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let now = Instant::now();

    // 1. Check RAM Cache
    if let Ok(guard) = DNS_CACHE.lock() {
        if let Some(ref cache) = *guard {
            if let Some(entry) = cache.get(target) {
                if entry.expires_at > now {
                    return Ok(entry.addresses.clone());
                }
            }
        }
    }

    // 2. Perform Network DNS Lookup with 2.5s Timeout
    let lookup_fut = tokio::net::lookup_host(target);
    let resolved: Vec<SocketAddr> = match tokio::time::timeout(Duration::from_millis(2500), lookup_fut).await {
        Ok(Ok(addrs)) => addrs.collect(),
        Ok(Err(e)) => return Err(Box::new(e)),
        Err(_) => return Err("DNS resolution timed out".into()),
    };

    if resolved.is_empty() {
        return Err("No IP addresses found for host".into());
    }

    // 3. Sort: Prioritize IPv4 for mobile dongles & residential carrier compatibility
    let mut sorted_addrs = resolved;
    sorted_addrs.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });

    // 4. Save to Cache with 60-second TTL
    if let Ok(mut guard) = DNS_CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        // Prune old entries if cache grows beyond 2000 hosts
        if cache.len() > 2000 {
            cache.retain(|_, v| v.expires_at > now);
        }
        cache.insert(
            target.to_string(),
            DnsCacheEntry {
                addresses: sorted_addrs.clone(),
                expires_at: now + Duration::from_secs(60),
            },
        );
    }

    Ok(sorted_addrs)
}

/// Proxy service configuration and lifecycle controller.
pub struct ProxyInstance {
    pub proxy_id: u64,
    pub port: u16,
    pub protocol: String,
    pub authorized_users: Arc<RwLock<HashMap<String, String>>>,
    pub bind_adapter_ip: Option<IpAddr>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub active_connections: Arc<AtomicU64>,
    pub bytes_transferred: Arc<AtomicU64>,
    pub is_running: Arc<AtomicBool>,
}

impl ProxyInstance {
    pub async fn start(
        proxy_id: u64,
        port: u16,
        protocol: String,
        authorized_users_list: Vec<crate::models::AuthorizedUser>,
        bind_adapter_ip: Option<IpAddr>,
    ) -> Result<Self, String> {
        let socket = TcpSocket::new_v4().map_err(|e| format!("Socket creation error: {}", e))?;
        let _ = socket.set_reuseaddr(true);
        #[cfg(unix)]
        let _ = socket.set_reuseport(true);

        socket
            .bind(SocketAddr::from(([0, 0, 0, 0], port)))
            .map_err(|e| format!("Failed to bind port {}: {}", port, e))?;

        let listener = socket
            .listen(2048)
            .map_err(|e| format!("Failed to listen on port {}: {}", port, e))?;

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let active_connections = Arc::new(AtomicU64::new(0));
        let bytes_transferred = Arc::new(AtomicU64::new(0));
        let is_running = Arc::new(AtomicBool::new(true));

        let mut user_map = HashMap::new();
        for u in authorized_users_list {
            if !u.username.trim().is_empty() {
                user_map.insert(u.username.trim().to_string(), u.password.trim().to_string());
            }
        }
        let authorized_users = Arc::new(RwLock::new(user_map));

        let mut shutdown_rx = shutdown_tx.subscribe();
        let running_flag = is_running.clone();
        let active_conns = active_connections.clone();
        let total_bytes = bytes_transferred.clone();
        let auth_creds = authorized_users.clone();
        let adapter_ip = bind_adapter_ip;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, client_addr)) => {
                                let _ = stream.set_nodelay(true);
                                let conns = active_conns.clone();
                                let bytes = total_bytes.clone();
                                let creds = auth_creds.clone();
                                let outbound_ip = adapter_ip;

                                conns.fetch_add(1, Ordering::SeqCst);

                                tokio::spawn(async move {
                                    let _ = handle_connection(stream, client_addr, proxy_id, creds, outbound_ip, bytes).await;
                                    conns.fetch_sub(1, Ordering::SeqCst);
                                });
                            }
                            Err(e) => {
                                log::warn!("Accept error on port {}: {}", port, e);
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        log::info!("Shutting down proxy listener on port {}", port);
                        running_flag.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            proxy_id,
            port,
            protocol,
            authorized_users,
            bind_adapter_ip,
            shutdown_tx,
            active_connections,
            bytes_transferred,
            is_running,
        })
    }

    pub fn update_authorized_users(&self, users: Vec<crate::models::AuthorizedUser>) {
        if let Ok(mut guard) = self.authorized_users.write() {
            guard.clear();
            for u in users {
                if !u.username.trim().is_empty() {
                    guard.insert(u.username.trim().to_string(), u.password.trim().to_string());
                }
            }
        }
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(());
        self.is_running.store(false, Ordering::SeqCst);
    }
}

/// Handle incoming client connection (SOCKS5 & HTTP Connect auto-detect)
async fn handle_connection(
    stream: TcpStream,
    client_addr: SocketAddr,
    proxy_id: u64,
    auth_creds: Arc<RwLock<HashMap<String, String>>>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = stream.set_nodelay(true);
    let mut peek_buf = [0u8; 3];
    let n = stream.peek(&mut peek_buf).await?;
    if n == 0 {
        return Ok(());
    }

    if peek_buf[0] == 0x05 {
        // SOCKS5 Protocol
        handle_socks5(stream, client_addr, proxy_id, auth_creds, outbound_ip, bytes_counter).await
    } else {
        // HTTP / HTTPS Connect Protocol
        handle_http_proxy(stream, client_addr, proxy_id, auth_creds, outbound_ip, bytes_counter).await
    }
}

/// SOCKS5 Handler
async fn handle_socks5(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    proxy_id: u64,
    auth_creds: Arc<RwLock<HashMap<String, String>>>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut ver_buf = [0u8; 2];
    stream.read_exact(&mut ver_buf).await?;
    let nmethods = ver_buf[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;

    let is_auth_required = {
        let guard = auth_creds.read().map_err(|_| "Lock poisoned")?;
        !guard.is_empty()
    };

    let mut client_user: Option<String> = None;

    if is_auth_required {
        // If credentials are required, client MUST authenticate with Username/Password (RFC 1929)
        if !methods.contains(&0x02) {
            // No acceptable auth methods
            stream.write_all(&[0x05, 0xFF]).await?;
            return Ok(());
        }

        // Tell client to authenticate via Username/Password (0x02)
        stream.write_all(&[0x05, 0x02]).await?;

        // Read subnegotiation version (RFC 1929: 0x01)
        let auth_ver = stream.read_u8().await?;
        if auth_ver != 0x01 {
            stream.write_all(&[0x01, 0xFF]).await?;
            return Ok(());
        }

        let ulen = stream.read_u8().await? as usize;
        let mut u_bytes = vec![0u8; ulen];
        stream.read_exact(&mut u_bytes).await?;
        let u = String::from_utf8_lossy(&u_bytes).trim().to_string();

        let plen = stream.read_u8().await? as usize;
        let mut p_bytes = vec![0u8; plen];
        stream.read_exact(&mut p_bytes).await?;
        let p = String::from_utf8_lossy(&p_bytes).trim().to_string();

        let is_valid = {
            let guard = auth_creds.read().map_err(|_| "Lock poisoned")?;
            guard.get(&u) == Some(&p)
        };

        if !is_valid {
            // RFC 1929: 0x01 (version), 0xFF (failure)
            stream.write_all(&[0x01, 0xFF]).await?;
            return Ok(());
        }

        client_user = Some(u);
        // RFC 1929: 0x01 (version), 0x00 (success)
        stream.write_all(&[0x01, 0x00]).await?;
    } else {
        // No authentication required
        if methods.contains(&0x00) {
            stream.write_all(&[0x05, 0x00]).await?;
        } else if methods.contains(&0x02) {
            stream.write_all(&[0x05, 0x02]).await?;
            let _ = stream.read_u8().await?;
            let ulen = stream.read_u8().await? as usize;
            let mut u_bytes = vec![0u8; ulen];
            stream.read_exact(&mut u_bytes).await?;
            let u = String::from_utf8_lossy(&u_bytes).trim().to_string();
            let plen = stream.read_u8().await? as usize;
            let mut p_bytes = vec![0u8; plen];
            stream.read_exact(&mut p_bytes).await?;
            client_user = Some(u);
            stream.write_all(&[0x01, 0x00]).await?;
        } else {
            stream.write_all(&[0x05, 0xFF]).await?;
            return Ok(());
        }
    }

    // Read Request
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    let cmd = header[1];
    let atyp = header[3];

    if cmd != 0x01 {
        // Only CONNECT supported
        stream.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }

    let target_host: String = match atyp {
        0x01 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let len = stream.read_u8().await? as usize;
            let mut domain = vec![0u8; len];
            stream.read_exact(&mut domain).await?;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => {
            stream.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };

    let target_port = stream.read_u16().await?;
    let target_addr_str = format!("{}:{}", target_host, target_port);

    // Outbound connection bound to the selected network interface / adapter IP
    let start_time = Instant::now();
    let outbound_stream = match connect_outbound(&target_addr_str, outbound_ip).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Outbound connection failed to {}: {}", target_addr_str, e);
            stream.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };

    // SOCKS5 success reply
    stream.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;

    // Bidirectional stream relay with 512KB high-speed stream buffers and active timing
    let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
    let duration_ms = start_time.elapsed().as_millis() as u64;

    record_log(ProxyLogEntry {
        proxy_id,
        username: client_user,
        source_ip: Some(client_addr.ip().to_string()),
        destination_host: Some(target_host),
        destination_port: Some(target_port),
        bytes_sent,
        bytes_received: bytes_recv,
        duration_ms,
        protocol: "socks5".to_string(),
    });

    Ok(())
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match TABLE.iter().position(|&x| x == b) {
            Some(idx) => idx as u32,
            None => return None,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// HTTP / HTTPS Connect Handler
async fn handle_http_proxy(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    proxy_id: u64,
    auth_creds: Arc<RwLock<HashMap<String, String>>>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");

    let is_auth_required = {
        let guard = auth_creds.read().map_err(|_| "Lock poisoned")?;
        !guard.is_empty()
    };

    let mut client_user: Option<String> = None;
    let mut detected_source_ip = client_addr.ip().to_string();
    let mut is_authenticated = !is_auth_required;

    for line in req_str.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();

        if lower.starts_with("proxy-authorization:") {
            if let Some(pos) = lower.find("basic ") {
                let encoded = trimmed[pos + 6..].trim();
                if let Some(decoded) = decode_base64(encoded) {
                    if let Ok(cred_str) = String::from_utf8(decoded) {
                        if let Some((u, p)) = cred_str.split_once(':') {
                            let u_clean = u.trim();
                            let p_clean = p.trim();
                            if is_auth_required {
                                let guard = auth_creds.read().map_err(|_| "Lock poisoned")?;
                                if guard.get(u_clean) == Some(&p_clean.to_string()) {
                                    is_authenticated = true;
                                    client_user = Some(u_clean.to_string());
                                }
                            } else {
                                client_user = Some(u_clean.to_string());
                            }
                        }
                    }
                }
            }
        } else if lower.starts_with("x-forwarded-for:") || lower.starts_with("x-real-ip:") {
            if let Some((_, ip_val)) = trimmed.split_once(':') {
                let first_ip = ip_val.split(',').next().unwrap_or("").trim();
                if !first_ip.is_empty() {
                    detected_source_ip = first_ip.to_string();
                }
            }
        }
    }

    if is_auth_required && !is_authenticated {
        // Return HTTP 407 Proxy Authentication Required
        let res = b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"ProxyPK\"\r\nContent-Type: text/plain\r\nContent-Length: 32\r\nConnection: close\r\n\r\nProxy Authentication Required.\r\n";
        let _ = stream.write_all(res).await;
        return Ok(());
    }

    if first_line.starts_with("CONNECT ") {
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Ok(());
        }
        let target_addr = parts[1];

        let start_time = Instant::now();
        match connect_outbound(target_addr, outbound_ip).await {
            Ok(outbound_stream) => {
                stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
                let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
                let duration_ms = start_time.elapsed().as_millis() as u64;

                let host_parts: Vec<&str> = target_addr.split(':').collect();
                let dest_host = host_parts.first().unwrap_or(&target_addr).to_string();
                let dest_port = host_parts.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(443);

                record_log(ProxyLogEntry {
                    proxy_id,
                    username: client_user,
                    source_ip: Some(detected_source_ip),
                    destination_host: Some(dest_host),
                    destination_port: Some(dest_port),
                    bytes_sent,
                    bytes_received: bytes_recv,
                    duration_ms,
                    protocol: "https".to_string(),
                });
            }
            Err(e) => {
                log::warn!("CONNECT outbound error to {}: {}", target_addr, e);
                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            }
        }
    } else {
        // Plain HTTP Request
        if let Some(host_line) = req_str.lines().find(|l| l.to_ascii_lowercase().starts_with("host:")) {
            let host = host_line.split_once(':').map(|(_, h)| h.trim()).unwrap_or("");
            let target_addr = if host.contains(':') {
                host.to_string()
            } else {
                format!("{}:80", host)
            };

            let start_time = Instant::now();
            if let Ok(mut outbound_stream) = connect_outbound(&target_addr, outbound_ip).await {
                let _ = outbound_stream.write_all(&buf[..n]).await;
                let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
                let duration_ms = start_time.elapsed().as_millis() as u64;

                record_log(ProxyLogEntry {
                    proxy_id,
                    username: client_user,
                    source_ip: Some(detected_source_ip),
                    destination_host: Some(host.to_string()),
                    destination_port: Some(80),
                    bytes_sent: bytes_sent + n as u64,
                    bytes_received: bytes_recv,
                    duration_ms,
                    protocol: "http".to_string(),
                });
            }
        }
    }

    Ok(())
}

/// Connect to target with in-memory DNS caching, OS dynamic TCP window scaling, and interface binding
async fn connect_outbound(
    target: &str,
    bind_ip: Option<IpAddr>,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let addrs = resolve_host_cached(target).await?;

    let mut last_err = None;

    for target_addr in &addrs {
        if let Some(local_ip) = bind_ip {
            let connect_fut = async {
                match (local_ip, target_addr) {
                    (IpAddr::V4(v4), SocketAddr::V4(_)) => {
                        if let Ok(s) = TcpSocket::new_v4() {
                            let _ = s.set_reuseaddr(true);
                            #[cfg(unix)]
                            let _ = s.set_reuseport(true);
                            let _ = s.set_nodelay(true);
                            if s.bind(SocketAddr::new(IpAddr::V4(v4), 0)).is_ok() {
                                s.connect(*target_addr).await.ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    (IpAddr::V6(v6), SocketAddr::V6(_)) => {
                        if let Ok(s) = TcpSocket::new_v6() {
                            let _ = s.set_reuseaddr(true);
                            #[cfg(unix)]
                            let _ = s.set_reuseport(true);
                            let _ = s.set_nodelay(true);
                            if s.bind(SocketAddr::new(IpAddr::V6(v6), 0)).is_ok() {
                                s.connect(*target_addr).await.ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            };

            // Fast 2.5s timeout per IP handshake to prevent connection stalls
            if let Ok(Some(stream)) = tokio::time::timeout(Duration::from_millis(2500), connect_fut).await {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            } else {
                last_err = Some(std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!("Outbound connection via adapter IP {} timed out or failed", local_ip),
                ));
            }
        } else {
            // Direct connect fallback with 2.5s timeout only when no specific adapter is bound
            if let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_millis(2500), TcpStream::connect(target_addr)).await {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            } else {
                last_err = Some(std::io::Error::new(std::io::ErrorKind::TimedOut, "TCP connect timed out"));
            }
        }
    }

    Err(last_err
        .map(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        .unwrap_or_else(|| "Connection to all resolved target IPs failed".into()))
}

/// Commercial-grade bidirectional data transfer using 512KB (524,288 bytes) high-speed stream buffers.
/// Supports active transfer throughput tracking and fast teardown on socket disconnects.
async fn relay_streams(
    mut client: TcpStream,
    mut target: TcpStream,
    bytes_counter: Arc<AtomicU64>,
) -> (u64, u64) {
    let _ = client.set_nodelay(true);
    let _ = target.set_nodelay(true);

    // 512 KB Buffer (524,288 bytes) per direction for maximum bulk data throughput
    const RELAY_BUFFER_SIZE: usize = 524288;

    match tokio::io::copy_bidirectional_with_sizes(&mut client, &mut target, RELAY_BUFFER_SIZE, RELAY_BUFFER_SIZE).await {
        Ok((from_client, from_target)) => {
            bytes_counter.fetch_add(from_client + from_target, Ordering::Relaxed);
            (from_client, from_target)
        }
        Err(_) => (0, 0),
    }
}
