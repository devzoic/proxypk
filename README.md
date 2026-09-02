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

### Linux (Ubuntu / Debian / AppImage)

#### AppImage (Portable)
```bash
chmod +x ProxyPK.Agent_*.AppImage
./ProxyPK.Agent_*.AppImage
```

#### Debian Package (.deb)
```bash
sudo dpkg -i ProxyPK.Agent_*_amd64.deb
sudo apt-get install -f
```

### Windows (10 / 11)
1. Download the `.exe` setup installer or `.msi` package.
2. Run the installer and follow the setup wizard.
3. Launch ProxyPK Agent from the Start Menu or Desktop shortcut.

---

## Building from Source

### Prerequisites
- [Node.js](https://nodejs.org/) (v18+) & `npm`
- [Rust](https://www.rust-lang.org/) toolchain (`rustup`)

### Steps
```bash
# 1. Clone repository
git clone https://github.com/devzoic/proxypk.git
cd proxypk

# 2. Install dependencies
npm install

# 3. Start development mode
npm run tauri dev

# 4. Build production bundle
npm run tauri build
```
The compiled installers will be generated under `src-tauri/target/release/bundle/`.

---

## License
All rights reserved.
