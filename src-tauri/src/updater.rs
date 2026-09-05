use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

static IS_UPDATING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, Clone)]
pub struct UpdateProgress {
    pub status: String,
    pub percent: f64,
    pub downloaded_mb: f64,
    pub total_mb: f64,
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Download and automatically install the latest release for current OS
#[tauri::command]
pub async fn download_and_install_update(
    app: AppHandle,
) -> Result<String, String> {
    if IS_UPDATING.swap(true, Ordering::SeqCst) {
        return Err("Update is already in progress.".to_string());
    }

    let res = execute_update(app.clone()).await;
    IS_UPDATING.store(false, Ordering::SeqCst);
    res
}

async fn execute_update(app: AppHandle) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("ProxyPK-Desktop-Agent-Updater")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let _ = app.emit(
        "update-progress",
        UpdateProgress {
            status: "Checking latest release assets...".to_string(),
            percent: 5.0,
            downloaded_mb: 0.0,
            total_mb: 0.0,
        },
    );

    let release_url = "https://api.github.com/repos/devzoic/proxypk/releases/latest";
    let resp = client
        .get(release_url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub release API: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned status: {}", resp.status()));
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse release metadata: {}", e))?;

    // Determine target asset for current OS
    let os = std::env::consts::OS;
    let target_asset = release.assets.iter().find(|asset| {
        let name = asset.name.to_lowercase();
        match os {
            "windows" => {
                name.ends_with(".exe") && (name.contains("setup") || name.contains("proxypk") || name.contains("agent"))
            }
            "linux" => name.ends_with(".appimage"),
            "macos" => name.ends_with(".dmg") || name.ends_with(".app.tar.gz"),
            _ => false,
        }
    }).or_else(|| {
        // Fallback search
        release.assets.iter().find(|asset| {
            let name = asset.name.to_lowercase();
            match os {
                "windows" => name.ends_with(".exe") || name.ends_with(".msi"),
                "linux" => name.ends_with(".deb") || name.ends_with(".appimage"),
                "macos" => name.ends_with(".dmg"),
                _ => false,
            }
        })
    }).ok_or_else(|| format!("No compatible update package found for {} ({})", os, release.tag_name))?;

    let target_url = target_asset.browser_download_url.clone();
    let total_bytes = target_asset.size;
    let total_mb = (total_bytes as f64) / (1024.0 * 1024.0);

    let temp_dir = std::env::temp_dir().join("proxypk_updates");
    let _ = std::fs::create_dir_all(&temp_dir);
    let downloaded_file_path = temp_dir.join(&target_asset.name);

    let _ = app.emit(
        "update-progress",
        UpdateProgress {
            status: format!("Downloading {}...", target_asset.name),
            percent: 10.0,
            downloaded_mb: 0.0,
            total_mb,
        },
    );

    // Stream download with progress reporting
    let mut download_resp = client
        .get(&target_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download update binary: {}", e))?;

    let mut file = File::create(&downloaded_file_path)
        .map_err(|e| format!("Failed to create update file: {}", e))?;

    let mut downloaded_bytes: u64 = 0;
    while let Some(chunk) = download_resp
        .chunk()
        .await
        .map_err(|e| format!("Error while downloading: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Failed to write chunk: {}", e))?;
        downloaded_bytes += chunk.len() as u64;

        let pct = if total_bytes > 0 {
            ((downloaded_bytes as f64 / total_bytes as f64) * 80.0) + 10.0
        } else {
            50.0
        };

        let downloaded_mb = (downloaded_bytes as f64) / (1024.0 * 1024.0);
        let _ = app.emit(
            "update-progress",
            UpdateProgress {
                status: format!("Downloading update: {:.1} MB / {:.1} MB ({:.0}%)", downloaded_mb, total_mb, pct),
                percent: pct.min(95.0),
                downloaded_mb,
                total_mb,
            },
        );
    }
    file.flush().map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    let _ = app.emit(
        "update-progress",
        UpdateProgress {
            status: "Download completed. Installing and restarting...".to_string(),
            percent: 98.0,
            downloaded_mb: total_mb,
            total_mb,
        },
    );

    // Platform specific installation & restart
    install_and_restart(app, downloaded_file_path, os)?;

    Ok(format!("Update {} installed successfully.", release.tag_name))
}

fn install_and_restart(
    _app: AppHandle,
    installer_path: PathBuf,
    os: &str,
) -> Result<(), String> {
    match os {
        "windows" => {
            // Direct native execution bypassing cmd.exe / start "" quoting bugs
            let ext = installer_path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

            if ext == "msi" {
                let mut cmd = std::process::Command::new("msiexec");
                cmd.arg("/i")
                    .arg(&installer_path)
                    .arg("/qn")
                    .arg("/norestart");

                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000 | 0x00000200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
                }

                cmd.spawn().map_err(|e| format!("Failed to spawn MSI installer: {}", e))?;
            } else {
                // NSIS Setup Installer (e.g. ProxyPK.Agent_x64-setup.exe)
                let mut cmd = std::process::Command::new(&installer_path);
                cmd.arg("/S"); // NSIS 100% silent flag

                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000 | 0x00000200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
                }

                cmd.spawn().map_err(|e| format!("Failed to spawn setup installer: {}", e))?;
            }

            // Brief sleep to allow installer process initialization before exiting current agent
            std::thread::sleep(std::time::Duration::from_millis(600));
            std::process::exit(0);
        }
        "linux" => {
            #[cfg(unix)]
            {
                let ext = installer_path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

                if ext == "deb" {
                    // Debian package installer via pkexec
                    let _ = std::process::Command::new("pkexec")
                        .args(&["dpkg", "-i", installer_path.to_str().unwrap_or("")])
                        .spawn()
                        .map_err(|e| format!("Failed to run dpkg installer: {}", e))?;

                    std::thread::sleep(std::time::Duration::from_millis(500));
                    std::process::exit(0);
                } else {
                    // AppImage / ELF binary execution with atomic inode replacement
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&installer_path)
                        .map_err(|e| format!("Failed to read metadata: {}", e))?
                        .permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&installer_path, perms)
                        .map_err(|e| format!("Failed to set executable permissions: {}", e))?;

                    // Determine active AppImage or binary path
                    let appimage_env = std::env::var("APPIMAGE").ok();
                    let target_dest = if let Some(ref path_str) = appimage_env {
                        PathBuf::from(path_str)
                    } else if let Ok(current_exe) = std::env::current_exe() {
                        current_exe
                    } else {
                        installer_path.clone()
                    };

                    if target_dest.exists() && target_dest != installer_path {
                        let dest_dir = target_dest.parent().unwrap_or_else(|| std::path::Path::new("/tmp"));
                        let filename = target_dest.file_name().and_then(|n| n.to_str()).unwrap_or("proxypk-agent.AppImage");
                        let temp_dest = dest_dir.join(format!(".{}.ota_tmp", filename));

                        // 1. Copy downloaded payload to temp file on SAME directory/filesystem
                        std::fs::copy(&installer_path, &temp_dest)
                            .map_err(|e| format!("Failed to stage update in {}: {}", dest_dir.display(), e))?;

                        // 2. Grant execution permissions on staged file
                        if let Ok(meta) = std::fs::metadata(&temp_dest) {
                            let mut p = meta.permissions();
                            p.set_mode(0o755);
                            let _ = std::fs::set_permissions(&temp_dest, p);
                        }

                        // 3. Atomically replace active file (Linux permits rename/unlink over running executable)
                        if let Err(rename_err) = std::fs::rename(&temp_dest, &target_dest) {
                            log::warn!("Direct rename failed ({}), unlinking old binary before move...", rename_err);
                            let _ = std::fs::remove_file(&target_dest);
                            std::fs::rename(&temp_dest, &target_dest)
                                .map_err(|e| format!("Failed to atomically replace active binary at {}: {}", target_dest.display(), e))?;
                        }

                        // 4. Launch updated AppImage with clean environment
                        let mut cmd = std::process::Command::new(&target_dest);
                        cmd.env_remove("APPDIR");
                        cmd.env_remove("OWD");
                        cmd.spawn()
                            .map_err(|e| format!("Failed to restart updated AppImage: {}", e))?;

                        std::thread::sleep(std::time::Duration::from_millis(500));
                        std::process::exit(0);
                    } else {
                        // Direct launch from downloaded installer path
                        let mut cmd = std::process::Command::new(&installer_path);
                        cmd.env_remove("APPDIR");
                        cmd.env_remove("OWD");
                        cmd.spawn()
                            .map_err(|e| format!("Failed to launch updated binary: {}", e))?;

                        std::thread::sleep(std::time::Duration::from_millis(500));
                        std::process::exit(0);
                    }
                }
            }

            #[cfg(not(unix))]
            {
                return Err("Linux update execution unsupported on non-unix".to_string());
            }
        }
        "macos" => {
            let is_tar_gz = installer_path.to_string_lossy().ends_with(".tar.gz");
            if is_tar_gz {
                // Extract app bundle directly into /Applications
                let _ = std::process::Command::new("tar")
                    .args(&["-xzf", installer_path.to_str().unwrap_or(""), "-C", "/Applications"])
                    .output();

                std::process::Command::new("open")
                    .args(&["-n", "/Applications/ProxyPK Agent.app"])
                    .spawn()
                    .map_err(|e| format!("Failed to open updated macOS app: {}", e))?;

                std::process::exit(0);
            } else {
                std::process::Command::new("open")
                    .arg(&installer_path)
                    .spawn()
                    .map_err(|e| format!("Failed to open macOS installer: {}", e))?;

                std::process::exit(0);
            }
        }
        _ => return Err(format!("Unsupported operating system: {}", os)),
    }
}
