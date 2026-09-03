# ProxyPK Desktop Agent

Official desktop agent application for ProxyPK.

[![Release](https://img.shields.io/github/v/release/devzoic/proxypk?color=0AB68A&label=Release)](https://github.com/devzoic/proxypk/releases)
[![Build Status](https://img.shields.io/github/actions/workflow/status/devzoic/proxypk/release.yml?color=0AB68A)](https://github.com/devzoic/proxypk/actions)

---

## Installation

Download the latest installer for your operating system from [**GitHub Releases**](https://github.com/devzoic/proxypk/releases/latest).

### macOS (Apple Silicon & Intel)
1. Download the `.dmg` installer (`aarch64` for Apple Silicon or `x86_64` for Intel).
2. Open the `.dmg` file and drag **ProxyPK Agent** into your **Applications** folder.
3. Launch the application from Applications or Spotlight.

### Linux (Lubuntu / Ubuntu / Debian / Chromebook)

#### Option 1: Debian Package (.deb) — Recommended
```bash
# 1. Download the latest .deb package directly from GitHub Releases
wget https://github.com/devzoic/proxypk/releases/latest/download/proxypk-agent_amd64.deb -O proxypk.deb

# 2. Install package (apt automatically installs required dependencies like WebKitGTK)
sudo apt update
sudo apt install -y ./proxypk.deb

# 3. Launch ProxyPK
proxypk-agent
```

#### Option 2: Portable AppImage (.AppImage)
```bash
# 1. Download AppImage
wget https://github.com/devzoic/proxypk/releases/latest/download/proxypk-agent_amd64.AppImage -O ProxyPK.AppImage

# 2. Make executable and run
chmod +x ProxyPK.AppImage
./ProxyPK.AppImage
```
> *Note for Lubuntu / Ubuntu 24.04: If AppImage prompts for FUSE, run `sudo apt install -y libfuse2`.*

### Windows (10 / 11)
1. Download the `.exe` setup installer or `.msi` package.
2. Run the installer and follow the setup wizard.
3. Launch ProxyPK Agent from the Start Menu or Desktop shortcut.

---

## Building from Source

### Prerequisites

#### 1. System Dependencies (Lubuntu / Ubuntu / Debian / Chromebook)
On Debian/Ubuntu-based distributions like **Lubuntu** (or Chromebook with Crostini Linux container / native Lubuntu):

```bash
sudo apt update
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev \
  libxdo-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  pkg-config
```

#### 2. Rust Toolchain
Install the latest Rust toolchain:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

#### 3. Node.js & npm (v18+)
```bash
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs
```

---

### Step-by-Step Build Guide

```bash
# 1. Clone the repository
git clone https://github.com/devzoic/proxypk.git
cd proxypk/desktop

# 2. Install JavaScript/Tauri CLI dependencies
npm install

# 3. Run in Development Mode (Live Hot-Reload)
npm run tauri dev

# 4. Build Production Packages (.deb & .AppImage)
npm run tauri build
```

The compiled Linux binaries and packages will be located in:
- **Debian package**: `desktop/src-tauri/target/release/bundle/deb/`
- **AppImage package**: `desktop/src-tauri/target/release/bundle/appimage/`
- **Direct binary**: `desktop/src-tauri/target/release/proxypk-agent`

---

## License
All rights reserved. ProxyPK Team.
