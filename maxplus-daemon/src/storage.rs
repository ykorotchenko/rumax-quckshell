use std::path::Path;
use anyhow::Result;
use sqlx::sqlite::SqlitePool;
use serde::{Serialize, Deserialize};

#[derive(Debug)]
pub struct MaxPlusStorage {
    pool: Option<SqlitePool>,
}

impl MaxPlusStorage {
    pub fn new(db_path: Option<std::path::PathBuf>) -> Self {
        let pool = match db_path {
            Some(path) => {
                let pool = match SqlitePool::connect(&format!("sqlite:{}", path.display())) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to open SQLite database: {}", e);
                        return MaxPlusStorage { pool: None };
                    }
                };
                MaxPlusStorage { pool: Some(pool) }
            }
            None => {
                // In-memory database for testing
                let pool = match SqlitePool::connect("sqlite:memory:") {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to create in-memory SQLite: {}", e);
                        return MaxPlusStorage { pool: None };
                    }
                };
                MaxPlusStorage { pool: Some(pool) }
            }
        };
        
        // Initialize schema if pool is Some
        if let Some(ref pool) = pool {
            // We'll initialize tables on first use
        }
        
        pool
    }

    // Chat-related operations
    pub async fn save_chat(&self, chat_id: i64, title: &str, last_message: &str, unread: i32) -> Result<()> {
        if let Some(ref pool) = self.pool {
            sqlx::query!(
                "INSERT OR REPLACE INTO chats (chat_id, title, last_message, unread_count) VALUES (?, ?, ?, ?)",
                chat_id,
                title,
                last_message,
                unread
            )
            .execute(pool)
            .await
            .map(|_| ())?;
        }
        Ok(())
    }

    pub async fn get_chats(&self) -> Result<Vec<ChatMessageSummary>> {
        if let Some(ref pool) = self.pool {
            let rows = sqlx::query_as!(
                ChatMessageSummary,
                "SELECT chat_id, title, last_message, unread_count FROM chats ORDER BY last_message_time DESC"
            )
            .fetch_all(pool)
            .await?;
            Ok(rows)
        } else {
            Ok(vec![])
        }
    }

    // Message-related operations
    pub async fn save_message(&self, chat_id: i64, message_id: i64, text: &str, timestamp: i64, read: bool) -> Result<()> {
        if let Some(ref pool) = self.pool {
            sqlx::query!(
                "INSERT OR IGNORE INTO messages (chat_id, message_id, text, timestamp, read) VALUES (?, ?, ?, ?, ?)",
                chat_id,
                message_id,
                text,
                timestamp,
                read as i32
            )
            .execute(pool)
            .await
            .map(|_| ())?;
        }
        Ok(())
    }

    pub async fn get_messages(&self, chat_id: i64, limit: usize) -> Result<Vec<ChatMessageSummary>> {
        if let Some(ref pool) = self.pool {
            let rows = sqlx::query_as!(
                ChatMessageSummary,
                "SELECT chat_id, message_id as \"message_id: i64\", text, timestamp, read FROM messages WHERE chat_id = ? ORDER BY timestamp DESC LIMIT ?",
                chat_id,
                (limit as i64)
            )
            .fetch_all(pool)
            .await?;
            Ok(rows)
        } else {
            Ok(vec![])
        }
    }

    // Contact-related operations
    pub async fn save_contact(&self, contact_id: u64, name: &str, phone: Option<&str>) -> Result<()> {
        if let Some(ref pool) = self.pool {
            sqlx::query!(
                "INSERT OR REPLACE INTO contacts (contact_id, name, phone) VALUES (?, ?, ?)",
                contact_id,
                name,
                phone
            )
            .execute(pool)
            .await
            .map(|_| ())?;
        }
        Ok(())
    }

    pub async fn get_contacts(&self) -> Result<Vec<ContactSummary>> {
        if let Some(ref pool) = self.pool {
            let rows = sqlx::query_as!(
                ContactSummaryDb,
                "SELECT contact_id, name, phone FROM contacts ORDER BY name"
            )
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(|c| ContactSummary { id: c.id as u64, name: c.name, last_seen: 0 }).collect())
        } else {
            Ok(vec![])
        }
    }
}

// Database schema definition
pub const INIT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS chats (
    chat_id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    last_message TEXT,
    unread_count INTEGER DEFAULT 0,
    last_message_time INTEGER DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    text TEXT,
    timestamp INTEGER NOT NULL,
    read INTEGER DEFAULT 0,
    FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
);

CREATE TABLE IF NOT EXISTS contacts (
    contact_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    phone TEXT
);
"#;

// Internal DB types
#[derive(serde::Serialize, serde::Deserialize, sqlx::FromRow)]
struct ChatMessageSummary {
    chat_id: i64,
    message_id: i64,
    text: String,
    timestamp: i64,
    read: i32,
}

#[derive(serde::Serialize, serde::Deserialize, sqlx::FromRow)]
struct ContactSummaryDb {
    contact_id: i64,
    name: String,
    phone: Option<String>,
}