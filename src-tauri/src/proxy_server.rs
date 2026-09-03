use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::broadcast;

/// Structured connection log entry for high-performance batch shipping
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyLogEntry {
    pub proxy_id: u64,
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
        if logs.len() < 5000 {
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

/// Proxy service configuration and lifecycle controller.
pub struct ProxyInstance {
    pub proxy_id: u64,
    pub port: u16,
    pub protocol: String,
    pub username: Option<String>,
    pub password: Option<String>,
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
        username: Option<String>,
        password: Option<String>,
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
            .listen(1024)
            .map_err(|e| format!("Failed to listen on port {}: {}", port, e))?;

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let active_connections = Arc::new(AtomicU64::new(0));
        let bytes_transferred = Arc::new(AtomicU64::new(0));
        let is_running = Arc::new(AtomicBool::new(true));

        let mut shutdown_rx = shutdown_tx.subscribe();
        let running_flag = is_running.clone();
        let active_conns = active_connections.clone();
        let total_bytes = bytes_transferred.clone();
        let auth_user = username.clone();
        let auth_pass = password.clone();
        let adapter_ip = bind_adapter_ip;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, client_addr)) => {
                                let conns = active_conns.clone();
                                let bytes = total_bytes.clone();
                                let u = auth_user.clone();
                                let p = auth_pass.clone();
                                let outbound_ip = adapter_ip;

                                conns.fetch_add(1, Ordering::SeqCst);

                                tokio::spawn(async move {
                                    let _ = handle_connection(stream, client_addr, proxy_id, u, p, outbound_ip, bytes).await;
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
            username,
            password,
            bind_adapter_ip,
            shutdown_tx,
            active_connections,
            bytes_transferred,
            is_running,
        })
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
    auth_user: Option<String>,
    auth_pass: Option<String>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut peek_buf = [0u8; 3];
    let n = stream.peek(&mut peek_buf).await?;
    if n == 0 {
        return Ok(());
    }

    if peek_buf[0] == 0x05 {
        // SOCKS5 Protocol
        handle_socks5(stream, client_addr, proxy_id, auth_user, auth_pass, outbound_ip, bytes_counter).await
    } else {
        // HTTP / HTTPS Connect Protocol
        handle_http_proxy(stream, client_addr, proxy_id, auth_user, auth_pass, outbound_ip, bytes_counter).await
    }
}

/// SOCKS5 Handler
async fn handle_socks5(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    proxy_id: u64,
    auth_user: Option<String>,
    auth_pass: Option<String>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut ver_buf = [0u8; 2];
    stream.read_exact(&mut ver_buf).await?;
    let nmethods = ver_buf[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;

    let require_auth = auth_user.is_some() && auth_pass.is_some();

    if require_auth {
        if !methods.contains(&0x02) {
            stream.write_all(&[0x05, 0xFF]).await?;
            return Ok(());
        }
        stream.write_all(&[0x05, 0x02]).await?;

        // Read user/pass auth (RFC 1929)
        let mut auth_ver = [0u8; 1];
        stream.read_exact(&mut auth_ver).await?;
        let ulen = stream.read_u8().await? as usize;
        let mut u_bytes = vec![0u8; ulen];
        stream.read_exact(&mut u_bytes).await?;
        let u = String::from_utf8_lossy(&u_bytes);

        let plen = stream.read_u8().await? as usize;
        let mut p_bytes = vec![0u8; plen];
        stream.read_exact(&mut p_bytes).await?;
        let p = String::from_utf8_lossy(&p_bytes);

        if auth_user.as_deref() == Some(&u) && auth_pass.as_deref() == Some(&p) {
            stream.write_all(&[0x01, 0x00]).await?;
        } else {
            stream.write_all(&[0x01, 0x01]).await?;
            return Ok(());
        }
    } else {
        // No auth required
        stream.write_all(&[0x05, 0x00]).await?;
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

    // Bidirectional stream relay with timing and byte tracking
    let start_time = std::time::Instant::now();
    let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
    let duration_ms = start_time.elapsed().as_millis() as u64;

    record_log(ProxyLogEntry {
        proxy_id,
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

/// HTTP / HTTPS Connect Handler
async fn handle_http_proxy(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    proxy_id: u64,
    _auth_user: Option<String>,
    _auth_pass: Option<String>,
    outbound_ip: Option<IpAddr>,
    bytes_counter: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");

    if first_line.starts_with("CONNECT ") {
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Ok(());
        }
        let target_addr = parts[1];

        match connect_outbound(target_addr, outbound_ip).await {
            Ok(outbound_stream) => {
                stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
                let start_time = std::time::Instant::now();
                let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
                let duration_ms = start_time.elapsed().as_millis() as u64;

                let host_parts: Vec<&str> = target_addr.split(':').collect();
                let dest_host = host_parts.first().unwrap_or(&target_addr).to_string();
                let dest_port = host_parts.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(443);

                record_log(ProxyLogEntry {
                    proxy_id,
                    source_ip: Some(client_addr.ip().to_string()),
                    destination_host: Some(dest_host),
                    destination_port: Some(dest_port),
                    bytes_sent,
                    bytes_received: bytes_recv,
                    duration_ms,
                    protocol: "https".to_string(),
                });
            }
            Err(e) => {
                log::warn!("HTTP CONNECT outbound failed: {}", e);
                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            }
        }
    } else {
        // Plain HTTP proxy: parse host and forward
        if let Some(host_line) = req_str.lines().find(|l| l.to_lowercase().starts_with("host:")) {
            let host = host_line.split(':').nth(1).unwrap_or("").trim();
            let target_addr = if host.contains(':') {
                host.to_string()
            } else {
                format!("{}:80", host)
            };

            if let Ok(mut outbound_stream) = connect_outbound(&target_addr, outbound_ip).await {
                let _ = outbound_stream.write_all(&buf[..n]).await;
                let start_time = std::time::Instant::now();
                let (bytes_sent, bytes_recv) = relay_streams(stream, outbound_stream, bytes_counter).await;
                let duration_ms = start_time.elapsed().as_millis() as u64;

                record_log(ProxyLogEntry {
                    proxy_id,
                    source_ip: Some(client_addr.ip().to_string()),
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

/// Connect to target with optional local adapter IP binding and multi-IP resolution fallback
async fn connect_outbound(
    target: &str,
    bind_ip: Option<IpAddr>,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host(target).await?.collect();
    if addrs.is_empty() {
        return Err("No DNS addresses resolved".into());
    }

    // Prioritize IPv4 for cellular dongle & residential compatibility
    addrs.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });

    let mut last_err = None;

    for target_addr in &addrs {
        if let Some(local_ip) = bind_ip {
            let bound_stream = match (local_ip, target_addr) {
                (IpAddr::V4(v4), SocketAddr::V4(_)) => {
                    if let Ok(s) = TcpSocket::new_v4() {
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
            };

            if let Some(stream) = bound_stream {
                return Ok(stream);
            }
        }

        // Connect directly if binding was not applicable or unbound
        match TcpStream::connect(target_addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err
        .map(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        .unwrap_or_else(|| "Connection to all resolved target IPs failed".into()))
}

/// Bidirectional data transfer returning (bytes_sent_by_client, bytes_received_by_client)
async fn relay_streams(
    mut client: TcpStream,
    mut target: TcpStream,
    bytes_counter: Arc<AtomicU64>,
) -> (u64, u64) {
    let _ = client.set_nodelay(true);
    let _ = target.set_nodelay(true);

    let (mut cr, mut cw) = client.split();
    let (mut tr, mut tw) = target.split();

    let bytes_a = bytes_counter.clone();
    let client_sent = Arc::new(AtomicU64::new(0));
    let sent_tracker = client_sent.clone();
    let client_to_target = async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match cr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    bytes_a.fetch_add(n as u64, Ordering::Relaxed);
                    sent_tracker.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
        let _ = tw.shutdown().await;
    };

    let bytes_b = bytes_counter.clone();
    let client_recv = Arc::new(AtomicU64::new(0));
    let recv_tracker = client_recv.clone();
    let target_to_client = async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match tr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    bytes_b.fetch_add(n as u64, Ordering::Relaxed);
                    recv_tracker.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
        let _ = cw.shutdown().await;
    };

    tokio::join!(client_to_target, target_to_client);
    (client_sent.load(Ordering::Relaxed), client_recv.load(Ordering::Relaxed))
}
