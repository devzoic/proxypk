# ProxyPK Desktop Node Agent 🌐

> High-performance residential proxy node daemon built with **Tauri v2**, **Rust**, and **Glassmorphic UI**. Converts local network interfaces (Wi-Fi, 4G/5G USB Wingles, Ethernet) into managed residential and mobile proxy endpoints.

[![Release](https://img.shields.io/github/v/release/devzoic/proxypk?color=0AB68A&label=Release)](https://github.com/devzoic/proxypk/releases)
[![Build Status](https://img.shields.io/github/actions/workflow/status/devzoic/proxypk/release.yml?color=0AB68A)](https://github.com/devzoic/proxypk/actions)
[![License](https://img.shields.io/badge/License-Proprietary-blue.svg)](LICENSE)

---

## ✨ Features

- **⚡ Zero-Prompt Auto-Connection**: Seamlessly connects to `http://proxy.test` in local development and `https://proxy.devzoic.com` in production with zero manual setup.
- **🔄 Dual Protocol Engine**: High-throughput **SOCKS5** (RFC 1928) and **HTTP/HTTPS CONNECT** tunneling on a single port or dedicated ports.
- **📶 Multi-Interface Binding**: Automatically scans and binds proxies to dedicated physical hardware interfaces (Huawei/ZTE 4G/5G Wingles, Wi-Fi adapters, and Ethernet NICs).
- **🛡️ Secure Token Authentication**: Built-in Basic Authentication and customer token isolation with dynamic IP authorization.
- **📊 Real-time Telemetry & Heartbeats**: Streams live latency, bandwidth usage (bytes uploaded/downloaded), and destination hosts to the central control plane.
- **🚀 Over-The-Air (OTA) Auto-Updater**: Cryptographically verified background updates with release changelog viewing and one-click in-app restart.
- **🎨 Jade & Obsidian Theme**: Premium glassmorphic interface with Dark and Light mode support.

---

## 📥 Installation

Download the latest binary for your operating system from [**GitHub Releases**](https://github.com/devzoic/proxypk/releases/latest).

### 🍏 macOS (Apple Silicon & Intel)
1. Download `ProxyPK.Agent_x.x.x_aarch64.dmg` (Apple Silicon M1/M2/M3) or `x86_64.dmg` (Intel).
2. Open the `.dmg` file and drag **ProxyPK Agent** into your **Applications** folder.
3. Launch **ProxyPK Agent**.

> **Note**: On first launch on macOS, if prompted with Gatekeeper, right-click the app and choose **Open**, or go to **System Settings → Privacy & Security** and click **Open Anyway**.

---

### 🐧 Linux (Ubuntu, Debian, Lubuntu, Mint)

#### Option 1: AppImage (Universal Portable)
```bash
# Make executable and launch
chmod +x ProxyPK.Agent_*.AppImage
./ProxyPK.Agent_*.AppImage
```

#### Option 2: Debian Package (.deb)
```bash
sudo dpkg -i ProxyPK.Agent_*_amd64.deb
sudo apt-get install -f # Fix any missing system dependencies
```

#### Required Linux Dependencies:
If running on a minimal distribution (e.g. Lubuntu / Ubuntu Server with GUI):
```bash
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-0 \
  libayatana-appindicator3-1 \
  librsvg2-common \
  curl
```

---

### 🪟 Windows (10 / 11)
1. Download `ProxyPK.Agent_x.x.x_x64-setup.exe` or `.msi`.
2. Run the installer and follow the setup wizard.
3. The ProxyPK Agent daemon will launch automatically in the background.

---

## 🛠️ Development & Building from Source

### Prerequisites
- [Node.js](https://nodejs.org/) (v18 or higher) & `npm`
- [Rust](https://www.rust-lang.org/) (latest stable toolchain):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

### 1. Clone the Repository
```bash
git clone https://github.com/devzoic/proxypk.git
cd proxypk
```

### 2. Install Dependencies
```bash
npm install
```

### 3. Run in Development Mode (with Hot-Reload)
```bash
npm run tauri dev
```

### 4. Build Production Binaries
```bash
npm run tauri build
```
The compiled installers will be located in:
- **macOS**: `src-tauri/target/release/bundle/dmg/`
- **Linux**: `src-tauri/target/release/bundle/deb/` and `src-tauri/target/release/bundle/appimage/`
- **Windows**: `src-tauri/target/release/bundle/msi/` and `src-tauri/target/release/bundle/nsis/`

---

## 📡 Control Plane Configuration

By default, the desktop agent connects automatically:
- **Local Development**: `http://proxy.test`
- **Production Cloud**: `https://proxy.devzoic.com`

You can customize the endpoint anytime in **Settings → ProxyPK Server Endpoint URL** or click the **Local Dev** / **Live Cloud** quick preset buttons.

---

## 🔄 Automated Updates

ProxyPK Agent uses the built-in Tauri v2 updater. When a new release tag is pushed to GitHub, the agent automatically detects the new release, notifies the user in the **Updates** tab, and allows seamless one-click upgrade and restart.

---

## 📄 License
Proprietary software. Developed for ProxyPK Control Plane and Distributed Residential Network.
