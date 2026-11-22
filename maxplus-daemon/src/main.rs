use anyhow::Result;
use log::{info, error};
use std::process::Command;
use std::time::Duration;

use rumax::MaxClient;
use maxplus_daemon::{MaxPlusDaemon, DaemonConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::init();
    
    info!("maxplus-daemon starting...");

    // Configure daemon
    let config = DaemonConfig {
        database_path: Some(std::path::PathBuf::from("/home/yuf/.maxplus/maxplus.db")),
    };

    // Create daemon instance
    let daemon = MaxPlusDaemon::new(config)?;
    
    // Start the daemon main loop
    daemon.run().await?;
    
    Ok(())
}

// Helper function to check if daemon is running
fn is_daemon_running() -> bool {
    // Check if the D-Bus service is available
    // or check for the process
    Command::new("dbus-send")
        .args(&["--session", "--dest=org.maxplus.Client", "--print-reply", "/org/maxplus/Client", "org.maxplus.Client.GetStatus"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}