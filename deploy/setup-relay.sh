#!/usr/bin/env bash
# ==============================================================================
# ProxyPK Central Relay Server (Rathole & Kernel BBR) - 1-Click High-Speed Installer
# Supports: Ubuntu 20.04+, Lubuntu, Debian 11+, AlmaLinux 9+, CentOS Stream
# ==============================================================================

set -euo pipefail

echo "============================================================"
echo " 🚀 ProxyPK High-Speed Central Gateway Relay Installer"
echo "============================================================"

# Check root
if [ "$EUID" -ne 0 ]; then
  echo "❌ Please run as root (sudo bash setup-relay.sh)"
  exit 1
fi

# 1. Install dependencies
echo "📦 Installing prerequisites..."
if command -v apt-get >/dev/null 2>&1; then
    apt-get update -y
    apt-get install -y curl wget tar gzip ufw ca-certificates
elif command -v dnf >/dev/null 2>&1; then
    dnf install -y curl wget tar gzip firewalld ca-certificates
fi

# 2. Kernel TCP BBR & High-Throughput Network Optimization
echo "⚡ Tuning Linux Kernel & Enabling Google BBR Congestion Control..."
modprobe tcp_bbr 2>/dev/null || true

cat <<EOF > /etc/sysctl.d/99-proxypk-speed.conf
# ProxyPK Enterprise High-Throughput Network Optimization
net.core.default_qdisc = fq
net.ipv4.tcp_congestion_control = bbr
net.ipv4.tcp_fastopen = 3

# Expanded Socket Receive/Send Buffers (32 MB max)
net.core.rmem_max = 33554432
net.core.wmem_max = 33554432
net.core.rmem_default = 1048576
net.core.wmem_default = 1048576
net.core.optmem_max = 2048576
net.ipv4.tcp_rmem = 4096 1048576 33554432
net.ipv4.tcp_wmem = 4096 1048576 33554432

# Connection Queue Backlogs for High-Concurrency Multi-Threading
net.ipv4.tcp_max_syn_backlog = 32768
net.core.netdev_max_backlog = 65536
net.core.somaxconn = 65535

# Fast Socket Recycling & Ephemeral Port Range
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 15
net.ipv4.tcp_keepalive_time = 180
net.ipv4.tcp_keepalive_probes = 4
net.ipv4.tcp_keepalive_intvl = 10
net.ipv4.ip_local_port_range = 1024 65535

# System File Descriptors
fs.file-max = 2097152
EOF

sysctl -p /etc/sysctl.d/99-proxypk-speed.conf 2>/dev/null || sysctl --system 2>/dev/null || true

# Maximize file descriptor limits for high connection concurrency
cat <<EOF > /etc/security/limits.d/99-proxypk.conf
* soft nofile 1048576
* hard nofile 1048576
root soft nofile 1048576
root hard nofile 1048576
EOF

# 3. Download Rathole High-Performance Binary
ARCH=$(uname -m)
case "$ARCH" in
    x86_64) RATHOLE_ARCH="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) RATHOLE_ARCH="aarch64-unknown-linux-gnu" ;;
    *) echo "❌ Unsupported architecture: $ARCH"; exit 1 ;;
esac

RATHOLE_VERSION="v0.5.0"
DOWNLOAD_URL="https://github.com/rapiz1/rathole/releases/download/${RATHOLE_VERSION}/rathole-${RATHOLE_ARCH}.zip"

echo "📥 Downloading Rathole ${RATHOLE_VERSION} for ${RATHOLE_ARCH}..."
mkdir -p /tmp/rathole-install
cd /tmp/rathole-install
curl -sSL -O "$DOWNLOAD_URL" || wget "$DOWNLOAD_URL"

if command -v unzip >/dev/null 2>&1; then
    unzip -q "rathole-${RATHOLE_ARCH}.zip"
else
    if command -v apt-get >/dev/null 2>&1; then apt-get install -y unzip; else dnf install -y unzip; fi
    unzip -q "rathole-${RATHOLE_ARCH}.zip"
fi

cp rathole /usr/local/bin/rathole
chmod +x /usr/local/bin/rathole
cd /
rm -rf /tmp/rathole-install

# 4. Create Configuration Directory
mkdir -p /etc/rathole

# Prompt or use defaults
read -p "Enter Tunnel Secret Token [default: proxypk-secret-token]: " TOKEN
TOKEN=${TOKEN:-proxypk-secret-token}

read -p "Enter Tunnel Control Port [default: 2333]: " CONTROL_PORT
CONTROL_PORT=${CONTROL_PORT:-2333}

# Optimized Server TOML (nodelay = true, high backlog)
cat <<EOF > /etc/rathole/server.toml
# ProxyPK High-Speed Rathole Server Configuration
[server]
bind_addr = "0.0.0.0:${CONTROL_PORT}"
default_token = "${TOKEN}"

# Default service for health checking
[server.services.health]
token = "${TOKEN}"
bind_addr = "0.0.0.0:20000"
type = "tcp"
nodelay = true
EOF

# 5. Create Systemd Service with High File Limits
cat <<EOF > /etc/systemd/system/rathole.service
[Unit]
Description=ProxyPK Rathole High-Speed Reverse Tunnel Server
After=network.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rathole --server /etc/rathole/server.toml
Restart=always
RestartSec=2
LimitNOFILE=1048576
LimitNPROC=512000

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable rathole
systemctl restart rathole

# 6. Open Firewall Ports
echo "🛡️ Opening firewall ports..."
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
    ufw allow ${CONTROL_PORT}/tcp comment 'ProxyPK Control Port'
    ufw allow 20000:30000/tcp comment 'ProxyPK Ingress Ports'
elif command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
    firewall-cmd --permanent --add-port=${CONTROL_PORT}/tcp
    firewall-cmd --permanent --add-port=20000-30000/tcp
    firewall-cmd --reload
fi

echo "============================================================"
echo " ✅ ProxyPK High-Speed Relay Server successfully installed!"
echo "============================================================"
echo " ⚡ Google BBR TCP Congestion Control : Active"
echo " ⚡ Kernel Buffer Auto-Tuning        : 32MB Max Scaled"
echo " ⚡ File Descriptor Limits            : 1,048,576"
echo " ⚡ Control Port                      : ${CONTROL_PORT}"
echo " ⚡ Secret Token                      : ${TOKEN}"
echo " ⚡ Port Range                        : 20000 - 30000"
echo ""
echo "👉 Next Step: In your ProxyPK Admin Settings (/admin/settings),"
echo "   Set 'Server Host' to your VPS Public IP or Domain!"
echo "============================================================"
