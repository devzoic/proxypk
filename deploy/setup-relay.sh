#!/usr/bin/env bash
# ==============================================================================
# ProxyPK Central Relay Server (Rathole) - 1-Click Installer
# Supports: Ubuntu 20.04+, Debian 11+, AlmaLinux 9+, CentOS Stream
# ==============================================================================

set -euo pipefail

echo "============================================================"
echo " 🚀 ProxyPK Central Gateway Relay Installer (Rathole)"
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
    apt-get install -y curl wget tar gzip ufw
elif command -v dnf >/dev/null 2>&1; then
    dnf install -y curl wget tar gzip firewalld
fi

# 2. Download Rathole binary
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

# 3. Create Configuration Directory
mkdir -p /etc/rathole

# Prompt or use defaults
read -p "Enter Tunnel Secret Token [default: proxypk-secret-token]: " TOKEN
TOKEN=${TOKEN:-proxypk-secret-token}

read -p "Enter Tunnel Control Port [default: 2333]: " CONTROL_PORT
CONTROL_PORT=${CONTROL_PORT:-2333}

# Initial Server TOML
cat <<EOF > /etc/rathole/server.toml
# ProxyPK Rathole Server Configuration
[server]
bind_addr = "0.0.0.0:${CONTROL_PORT}"
default_token = "${TOKEN}"

# Ingress services will be automatically registered dynamically
EOF

# 4. Create Systemd Service
cat <<EOF > /etc/systemd/system/rathole.service
[Unit]
Description=ProxyPK Rathole Reverse Tunnel Server
After=network.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/rathole --server /etc/rathole/server.toml
Restart=always
RestartSec=3
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable rathole
systemctl restart rathole

# 5. Open Firewall Ports
echo "🛡️ Opening firewall ports..."
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
    ufw allow ${CONTROL_PORT}/tcp
    ufw allow 20000:30000/tcp comment 'ProxyPK Ingress Ports'
elif command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
    firewall-cmd --permanent --add-port=${CONTROL_PORT}/tcp
    firewall-cmd --permanent --add-port=20000-30000/tcp
    firewall-cmd --reload
fi

echo "============================================================"
echo " ✅ ProxyPK Relay Server successfully installed & running!"
echo "============================================================"
echo " Control Port : ${CONTROL_PORT}"
echo " Secret Token : ${TOKEN}"
echo " Port Range   : 20000 - 30000 (Forwarded to your Desktop Dongles)"
echo ""
echo "👉 Next Step: In your ProxyPK Admin Settings (/admin/settings),"
echo "   Set 'Server Host' to your VPS Public IP or Domain!"
echo "============================================================"
