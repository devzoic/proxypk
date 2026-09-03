#!/usr/bin/env bash
# ==============================================================================
# ProxyPK Central Relay Server - Real-Time Auto-Sync Daemon Installer
# Automatically synchronizes /etc/rathole/server.toml from Laravel API in real-time
# ==============================================================================

set -euo pipefail

echo "============================================================"
echo " ⚡ ProxyPK Central Relay Real-Time Auto-Sync Installer"
echo "============================================================"

if [ "$EUID" -ne 0 ]; then
  echo "❌ Please run as root: sudo bash setup-relay-sync.sh"
  exit 1
fi

# Ask for settings
read -p "Enter Laravel Web App URL (e.g. https://panel.yourdomain.com): " API_URL
API_URL="${API_URL%/}" # Remove trailing slash

read -p "Enter Tunnel Secret Token [default: proxypk-secret-token]: " TOKEN
TOKEN=${TOKEN:-proxypk-secret-token}

mkdir -p /etc/rathole
mkdir -p /var/log/rathole

# 1. Create the sync runner script
cat << 'EOF' > /usr/local/bin/rathole-sync
#!/usr/bin/env bash
set -e

CONFIG_FILE="/etc/rathole/server.toml"
ENV_FILE="/etc/rathole/sync.env"

if [ -f "$ENV_FILE" ]; then
    source "$ENV_FILE"
fi

if [ -z "${API_URL:-}" ] || [ -z "${TOKEN:-}" ]; then
    exit 0
fi

ENDPOINT="${API_URL}/api/desktop/tunnel/server-config?token=${TOKEN}"
TEMP_FILE="/tmp/rathole_server_latest.toml"

# Download live config from Laravel
HTTP_CODE=$(curl -sSL -w "%{http_code}" -o "$TEMP_FILE" "$ENDPOINT" || echo "000")

if [ "$HTTP_CODE" -eq 200 ]; then
    # Validate non-empty
    if [ -s "$TEMP_FILE" ]; then
        CURRENT_MD5=""
        if [ -f "$CONFIG_FILE" ]; then
            CURRENT_MD5=$(md5sum "$CONFIG_FILE" | awk '{print $1}')
        fi
        NEW_MD5=$(md5sum "$TEMP_FILE" | awk '{print $1}')

        if [ "$CURRENT_MD5" != "$NEW_MD5" ]; then
            cp "$TEMP_FILE" "$CONFIG_FILE"
            echo "[$(date -Iseconds)] [Rathole-Sync] Config updated (MD5: $NEW_MD5). Hot-reload triggered." >> /var/log/rathole/sync.log
        fi
        
        # Send heartbeat
        curl -sSL -X POST -H "Content-Type: application/json" -d "{\"token\":\"${TOKEN}\"}" "${API_URL}/api/desktop/tunnel/relay-heartbeat" >/dev/null 2>&1 || true
    fi
fi

rm -f "$TEMP_FILE"
EOF

chmod +x /usr/local/bin/rathole-sync

# 2. Store credentials
cat << EOF > /etc/rathole/sync.env
API_URL="${API_URL}"
TOKEN="${TOKEN}"
EOF

chmod 600 /etc/rathole/sync.env

# 3. Create Systemd Service and Timer
cat << 'EOF' > /etc/systemd/system/rathole-sync.service
[Unit]
Description=ProxyPK Rathole Real-Time Config Synchronizer
After=network.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/rathole-sync
EOF

cat << 'EOF' > /etc/systemd/system/rathole-sync.timer
[Unit]
Description=Run ProxyPK Rathole Sync every 5 seconds
After=network.target

[Timer]
OnBootSec=5sec
OnUnitActiveSec=5sec
AccuracySec=1sec

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now rathole-sync.timer

# Run initial sync immediately
/usr/local/bin/rathole-sync || true

echo "============================================================"
echo " ✅ Real-Time Sync Daemon Installed & Running!"
echo " - Rathole config will auto-sync every 5 seconds from Laravel."
echo " - Any proxy added in Laravel will hot-reload live on the VPS."
echo "============================================================"
