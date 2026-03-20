use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Embedded channel.ts content — written to disk during `tuicr mcp-channel install`.
const CHANNEL_TS: &str = include_str!("../../mcp_channel/channel.ts");

/// Embedded package.json content.
const PACKAGE_JSON: &str = include_str!("../../mcp_channel/package.json");

/// Run an MCP channel subcommand (install or uninstall).
pub fn run_subcommand(subcmd: &str) -> anyhow::Result<()> {
    match subcmd {
        "install" => install(),
        "uninstall" => uninstall(),
        _ => {
            eprintln!("Unknown mcp-channel subcommand: {subcmd}");
            eprintln!("Valid subcommands: install, uninstall");
            std::process::exit(2);
        }
    }
}

/// Detect a Node.js-compatible runtime (bun, node, or deno).
/// Returns (command, install_args) or None.
fn detect_runtime() -> Option<(&'static str, Vec<&'static str>)> {
    // Prefer bun (fastest), then node/npm, then deno
    if Command::new("bun").arg("--version").output().is_ok() {
        return Some(("bun", vec!["install"]));
    }
    if Command::new("node").arg("--version").output().is_ok()
        && Command::new("npm").arg("--version").output().is_ok()
    {
        return Some(("node", vec![])); // npm install handled separately
    }
    if Command::new("deno").arg("--version").output().is_ok() {
        return Some(("deno", vec!["install"]));
    }
    None
}

/// Get the channel installation directory.
fn channel_dir() -> PathBuf {
    dirs_for_install().join("mcp-channel")
}

fn dirs_for_install() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("tuicr"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config").join("tuicr")
        })
}

fn install() -> anyhow::Result<()> {
    // Check for a compatible runtime
    let Some((runtime, _)) = detect_runtime() else {
        eprintln!("Error: A Node.js-compatible runtime is required for the MCP channel.");
        eprintln!("Install one of: bun (https://bun.sh), Node.js (https://nodejs.org), or Deno (https://deno.land)");
        std::process::exit(1);
    };

    let dir = channel_dir();
    println!("Installing MCP channel server to {}...", dir.display());

    // Create directory
    fs::create_dir_all(&dir)?;

    // Write files
    fs::write(dir.join("channel.ts"), CHANNEL_TS)?;
    fs::write(dir.join("package.json"), PACKAGE_JSON)?;

    // Install dependencies
    println!("Installing dependencies with {runtime}...");
    let install_status = if runtime == "node" {
        Command::new("npm")
            .arg("install")
            .current_dir(&dir)
            .status()?
    } else {
        Command::new(runtime)
            .arg("install")
            .current_dir(&dir)
            .status()?
    };

    if !install_status.success() {
        eprintln!("Error: Failed to install dependencies");
        std::process::exit(1);
    }

    // Determine the run command for .mcp.json
    let (run_cmd, run_args) = match runtime {
        "bun" => ("bun", vec![dir.join("channel.ts").to_string_lossy().to_string()]),
        "deno" => ("deno", vec!["run".to_string(), "--allow-all".to_string(), dir.join("channel.ts").to_string_lossy().to_string()]),
        _ => ("node", vec![dir.join("channel.ts").to_string_lossy().to_string()]),
    };

    // Update .mcp.json in current directory
    let mcp_json_path = PathBuf::from(".mcp.json");
    let mut mcp_config: serde_json::Value = if mcp_json_path.exists() {
        let content = fs::read_to_string(&mcp_json_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let servers = mcp_config
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert(serde_json::json!({}));

    let args_json: Vec<serde_json::Value> = run_args.iter().map(|a| serde_json::json!(a)).collect();
    servers.as_object_mut().unwrap().insert(
        "tuicr".to_string(),
        serde_json::json!({
            "command": run_cmd,
            "args": args_json,
        }),
    );

    fs::write(&mcp_json_path, serde_json::to_string_pretty(&mcp_config)?)?;

    println!("MCP channel installed successfully!");
    println!("  Runtime: {runtime}");
    println!("  Channel: {}", dir.join("channel.ts").display());
    println!("  Config:  {}", mcp_json_path.display());
    println!();
    println!("Run tuicr with --mcp-channel to enable direct Claude Code communication.");

    Ok(())
}

fn uninstall() -> anyhow::Result<()> {
    let dir = channel_dir();

    // Remove from .mcp.json
    let mcp_json_path = PathBuf::from(".mcp.json");
    if mcp_json_path.exists() {
        let content = fs::read_to_string(&mcp_json_path)?;
        if let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
                servers.remove("tuicr");
            }
            fs::write(&mcp_json_path, serde_json::to_string_pretty(&config)?)?;
            println!("Removed tuicr from {}", mcp_json_path.display());
        }
    }

    // Remove channel directory
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
        println!("Removed {}", dir.display());
    }

    println!("MCP channel uninstalled.");
    Ok(())
}
