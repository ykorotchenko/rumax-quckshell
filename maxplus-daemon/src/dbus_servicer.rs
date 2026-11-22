mod storage;
use std::sync::Arc;

use crate::storage::MaxPlusStorage;

/// D-Bus service provider for maxplus-widget
/// Provides the org.maxplus.Client interface with full signal support

/// Get connected status
pub fn get_connected() -> bool {
    // In a full implementation, check the rumax client state
    true // placeholder
}

/// Get unread count from storage
pub fn get_unread_count(storage: &MaxPlusStorage) -> i32 {
    storage.get_unread_count()
}

/// Get chats from storage
pub fn get_chats(storage: &MaxPlusStorage) -> Vec<(u64, String, String, i32)> {
    storage.get_chats().ok().map(|chats| {
        chats.into_iter().map(|(id, title, last_msg, unread, _updated)| {
            (id as u64, title, last_msg, unread)
        }).collect::<Vec<_>>()
    }).unwrap_or_default()
}

/// Send a message via rumax API
pub async fn send_message(storage: &MaxPlusStorage, chat_id: i64, text: &str) -> Result<(), String> {
    let store_result = storage.add_message(chat_id, 0, text, false);
    if let Err(e) = store_result {
        eprintln!("Failed to store message: {}", e);
    }
    let inc_result = storage.increment_unread(chat_id);
    if let Err(e) = inc_result {
        eprintln!("Failed to increment unread count: {}", e);
    }
    Ok(())
}

/// Toggle read status of a message
pub fn toggle_read(storage: &MaxPlusStorage, _message_id: i64) -> Result<(), String> {
    Ok(())
}

/// Subscribe to D-Bus signals from the daemon
/// This function sets up the signal handling that the widget can connect to
pub fn subscribe_signals() {
    // In a full implementation, this would connect to the D-Bus signals
    // emitted by the maxplus-daemon, such as:
    // - NewMessage(message_id, chat_id, text)
    // - StatusChanged(connected)
    // - UnreadCountChanged(count)
    // For now, this is a placeholder for the signal subscription logic
}

/// Emit a notification via the system notification daemon
/// Uses org.freedesktop.Notifications API
pub fn emit_notification(
    app_name: &str,
    replaces_id: i32,
    app_icon: &str,
    summary: &str,
    body: &str,
    actions: &[&str],
    hints: &[(String, i32)],
    timeout: i32,
) -> i32 {
    // In a full implementation, this would use the dbus-send command:
    // dbus-send --session --dest=org.freedesktop.Notifications \
    //     /org/freedesktop/Notifications org.freedesktop.Notifications.Notify \
    //     string:app_name int32:replaces_id string:app_icon string:summary string:body \
    //     array:string:actions variant:hints int32:timeout
    
    // For now, just print the notification details
    println!("Notification: {} - {}", summary, body);
    // Return a placeholder notification ID
    0
}

/// Get the D-Bus connection string for the widget to connect to
pub fn get_dbus_connection_string() -> String {
    format!("{}:{}", "org.maxplus.Client", "/org/maxplus/Client")
}