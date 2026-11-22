use dbus::{Property, TreeNode};
use dbus::message::Signal;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MaxPlusDbusServicer {
    client: Arc<rumax::MaxClient>,
    storage: Arc<crate::storage::MaxPlusStorage>,
}

impl MaxPlusDbusServicer {
    pub fn new(client: &rumax::MaxClient, storage: &crate::storage::MaxPlusStorage) -> Self {
        MaxPlusDbusServicer {
            client: Arc::new(client.clone()),
            storage: Arc::new(storage.clone()),
        }
    }
}

// D-Bus interface methods for the widget

// Status methods
pub const GET_STATUS: &str = "GetStatus";
pub const GET_UNREAD_COUNT: &str = "GetUnreadCount";
pub const GET_CONTACTS: &str = "GetContacts";
pub const GET_CHATS: &str = "GetChats";
pub const SEND_MESSAGE: &str = "SendMessage";
pub const OPEN_CHAT: &str = "OpenChat";
pub const TOGGLE_READ: &str = "ToggleRead";

// Property keys
pub const PROP_CONNECTED: &str = "Connected";
pub const PROP_UNREAD_COUNT: &str = "UnreadCount";
pub const PROP_CURRENT_CHAT: &str = "CurrentChat";
pub const PROP_AVATAR_URL: &str = "AvatarUrl";

impl MaxPlusDbusServicer {
    // Get connected status
    pub fn get_connected(&self) -> bool {
        // Check if the rumax client has an active WebSocket connection
        // This is a simplified check - actual implementation would use the client state
        true // placeholder
    }

    // Get unread count across all chats
    pub fn get_unread_count(&self) -> usize {
        // Query storage for unread messages
        0 // placeholder
    }

    // Get contacts list
    pub fn get_contacts(&self) -> Vec<ContactSummary> {
        // Fetch from storage or request from rumax
        vec![] // placeholder
    }

    // Get recent chats
    pub fn get_chats(&self) -> Vec<ChatSummary> {
        // Fetch from SQLite storage
        vec![] // placeholder
    }

    // Send a message
    pub async fn send_message(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        // Use rumax to send the message
        let payload = serde_json::json!({
            "chatId": chat_id,
            "text": text
        });
        
        // This would use the rumax API
        // self.client.send_and_wait(...)?.await
        
        Ok(())
    }

    // Open a chat window
    pub fn open_chat(&self, chat_id: i64) -> anyhow::Result<()> {
        // Emit D-Bus signal or request widget to show chat
        Ok(())
    }

    // Toggle read status
    pub fn toggle_read(&self, message_id: i64) -> anyhow::Result<()> {
        // Update read status in storage
        Ok(())
    }
}

// Contact summary for D-Bus
#[derive(Debug, Clone, dbus::Type, PartialEq)]
#[dbus(type = ("ay", "q", "s"))] // signature: array of (uint, string)
pub struct ContactSummary {
    pub id: u64,
    pub name: String,
    pub last_seen: i64,
}

// Chat summary for D-Bus
#[derive(Debug, Clone, dbus::Type, PartialEq)]
#[dbus(type = ("q", "s", "s", "i"))] // signature: (uint, string, string, int)
pub struct ChatSummary {
    pub id: u64,
    pub title: String,
    pub last_message: String,
    pub unread_count: i32,
}

// Signal definitions
pub const SIGNAL_NEW_MESSAGE: &str = "NewMessage";
pub const SIGNAL_STATUS_CHANGED: &str = "StatusChanged";
pub const SIGNAL_UNREAD_COUNT_CHANGED: &str = "UnreadCountChanged";

#[derive(debug::Debug, Clone, dbus::Type)]
#[dbus(type = ("q", "s", "s"))] // signature: (uint, string, string) - message_id, chat_id, text
pub struct NewMessageSignal {
    pub message_id: u64,
    pub chat_id: i64,
    pub text: String,
}