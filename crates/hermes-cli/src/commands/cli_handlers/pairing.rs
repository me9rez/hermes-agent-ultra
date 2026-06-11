//! Pairing CLI handler.

use super::super::{mask_secret_value, secret_stdout_allowed};
pub async fn handle_cli_pairing(
    action: Option<String>,
    device_id: Option<String>,
    args: Vec<String>,
) -> Result<(), hermes_core::AgentError> {
    use crate::pairing_store::{PairingStatus, PairingStore};
    use hermes_gateway::DmPairingStore;

    let store = PairingStore::open_default();
    let dm_store = DmPairingStore::open_default();
    let action = action.unwrap_or_else(|| "list".to_string());

    match action.as_str() {
        "list" => {
            let devices = store.list().map_err(|e| hermes_core::AgentError::Io(e))?;
            if devices.is_empty() {
                println!("No paired devices.");
                println!("  Store: {}", PairingStore::default_path().display());
            } else {
                println!("Paired devices ({}):", devices.len());
                println!(
                    "  {:20} {:10} {:12} {}",
                    "Device ID", "Status", "Last Seen", "Name"
                );
                println!("  {}", "-".repeat(60));
                for d in &devices {
                    let last_seen = d.last_seen.as_deref().unwrap_or("never");
                    let name = d.name.as_deref().unwrap_or("(unnamed)");
                    let status_icon = match d.status {
                        PairingStatus::Pending => "⏳",
                        PairingStatus::Approved => "✓",
                        PairingStatus::Revoked => "✗",
                    };
                    println!(
                        "  {:20} {} {:8} {:12} {}",
                        d.device_id, status_icon, d.status, last_seen, name
                    );
                }
            }
            let pending = dm_store.list_pending(None);
            let approved = dm_store.list_approved(None);
            if pending.is_empty() && approved.is_empty() {
                println!("No DM pairing data found.");
            } else {
                if !pending.is_empty() {
                    println!("\nPending DM pairing requests ({}):", pending.len());
                    println!(
                        "  {:10} {:12} {:20} {:20} {}",
                        "Platform", "Code*", "User ID", "Name", "Age"
                    );
                    println!("  {}", "-".repeat(80));
                    for p in pending {
                        println!(
                            "  {:10} {:12} {:20} {:20} {}m",
                            p.platform, p.code, p.user_id, p.user_name, p.age_minutes
                        );
                    }
                    println!("  * code is hash prefix for display only");
                }
                if !approved.is_empty() {
                    println!("\nApproved DM users ({}):", approved.len());
                    println!("  {:10} {:24} {}", "Platform", "User ID", "Name");
                    println!("  {}", "-".repeat(60));
                    for a in approved {
                        println!("  {:10} {:24} {}", a.platform, a.user_id, a.user_name);
                    }
                }
            }
        }
        "approve" => {
            if let Some(did) = device_id {
                match store.approve(&did) {
                    Ok(dev) => {
                        println!("Device '{}' approved.", dev.device_id);
                        if let Some(secret) = &dev.shared_secret {
                            if secret_stdout_allowed() {
                                println!("  Shared secret: {}", secret);
                                println!(
                                    "  (plaintext output enabled via HERMES_ALLOW_SECRET_STDOUT=1)"
                                );
                            } else {
                                println!("  Shared secret: {}", mask_secret_value(secret));
                                println!(
                                    "  (set HERMES_ALLOW_SECRET_STDOUT=1 to reveal plaintext once)"
                                );
                            }
                            println!("  (Store this securely — it will not be shown again)");
                        }
                    }
                    Err(e) => println!("Failed to approve device: {}", e),
                }
            } else if args.len() >= 2 {
                let platform = &args[0];
                let code = &args[1];
                match dm_store
                    .approve_code(platform, code)
                    .map_err(hermes_core::AgentError::Io)?
                {
                    Some(user) => {
                        let display = if user.user_name.trim().is_empty() {
                            user.user_id.clone()
                        } else {
                            format!("{} ({})", user.user_name, user.user_id)
                        };
                        println!(
                            "Approved! User {} on {} can now use DM access.",
                            display, platform
                        );
                    }
                    None => {
                        println!(
                            "Code '{}' not found, expired, or locked out on '{}'.",
                            code, platform
                        );
                    }
                }
            } else {
                return Err(hermes_core::AgentError::Config(
                    "Missing args. Usage: hermes pairing approve --device-id <id> OR hermes pairing approve <platform> <code>".into(),
                ));
            }
        }
        "revoke" => {
            if let Some(did) = device_id {
                match store.revoke(&did) {
                    Ok(dev) => {
                        println!("Device '{}' revoked.", dev.device_id);
                        println!("  The device will no longer be able to connect.");
                    }
                    Err(e) => println!("Failed to revoke device: {}", e),
                }
            } else if args.len() >= 2 {
                let platform = &args[0];
                let user_id = &args[1];
                let revoked = dm_store
                    .revoke(platform, user_id)
                    .map_err(hermes_core::AgentError::Io)?;
                if revoked {
                    println!("Revoked DM access for {} on {}.", user_id, platform);
                } else {
                    println!("User {} was not approved on {}.", user_id, platform);
                }
            } else {
                return Err(hermes_core::AgentError::Config(
                    "Missing args. Usage: hermes pairing revoke --device-id <id> OR hermes pairing revoke <platform> <user_id>".into(),
                ));
            }
        }
        "clear-pending" => {
            match store.clear_pending() {
                Ok(count) => {
                    if count == 0 {
                        println!("No pending pairing requests to clear.");
                    } else {
                        println!("Cleared {} pending pairing request(s).", count);
                    }
                }
                Err(e) => println!("Failed to clear pending requests: {}", e),
            }
            let platform = args.first().map(|s| s.as_str());
            match dm_store.clear_pending(platform) {
                Ok(count) => {
                    if platform.is_some() {
                        println!("Cleared {} pending DM requests.", count);
                    } else {
                        println!(
                            "Cleared {} pending DM requests across all platforms.",
                            count
                        );
                    }
                }
                Err(e) => println!("Failed to clear DM pending requests: {}", e),
            }
        }
        other => {
            println!("Pairing action '{}' is not recognized.", other);
            println!("Available actions: list, approve, revoke, clear-pending");
        }
    }
    Ok(())
}
