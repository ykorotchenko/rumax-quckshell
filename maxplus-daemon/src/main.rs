use anyhow::Result;
use std::env;

use rumax::MaxClient;
use maxplus_daemon::{MaxPlusDaemon, DaemonConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Default database path
    let default_db_path = "/home/yuf/.maxplus/maxplus.db";

    // Configure daemon
    let config = DaemonConfig {
        database_path: Some(default_db_path.to_string()),
        phone_number: None,
        auth_token: None,
    };

    // Create daemon instance
    let mut daemon = maxplus_daemon::MaxPlusDaemon::new(config)?;

    // Check for auth command
    if args.len() > 1 {
        match args[1].as_str() {
            "auth" => {
                // Start authentication flow
                if args.len() > 2 {
                    // Phone number provided as argument
                    daemon.authenticate(args[2].clone()).await?;
                } else {
                    // Prompt for phone number
                    println!("Enter phone number for Max+ authentication:");
                    let mut phone = String::new();
                    std::io::stdin().read_line(&mut phone).map_err(|e| anyhow::anyhow!("Failed to read: {}", e))?;
                    daemon.authenticate(phone.trim().to_string()).await?;
                }
            }
            "status" => {
                // Show daemon status
                println!("Authentication status: {:?}", daemon.auth_status());
                if daemon.is_authenticated() {
                    println!("Auth token: {:?}", daemon.auth_token());
                }
                println!("D-Bus connection: {:?}", daemon.dbus_connection);
            }
            "help" => {
                println!("Max+ Daemon Commands:");
                println!("  auth [phone_number] - Authenticate with Max+");
                println!("  status              - Show daemon status");
                println!("  help                - Show this help message");
            }
            _ => {
                println!("Unknown command: {}", args[1]);
                println!("Use 'maxplus-daemon help' for available commands");
            }
        }
    } else {
        // No command show status and exit
        println!("Max+ Daemon - Not authenticated");
        println!("Run 'maxplus-daemon auth [phone_number]' to authenticate");
    }

    Ok(())
}