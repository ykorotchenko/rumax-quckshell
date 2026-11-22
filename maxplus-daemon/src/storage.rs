use std::path::Path;

/// SQLite storage for maxplus-daemon
/// Stores chat history, messages, contacts, and metadata

use rusqlite::Connection;

pub struct MaxPlusStorage {
    connection: Option<Connection>,
}

impl MaxPlusStorage {
    pub fn new(db_path: &Option<String>) -> Self {
        let connection = match db_path {
            Some(path) => {
                // Ensure directory exists
                if let Some(parent) = Path::new(path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                
                let conn = match Connection::open(path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Failed to open database at {:?}: {}", path, e);
                        return MaxPlusStorage { connection: None };
                    }
                };
                
                // Initialize schema
                let _ = conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS chats (
                        chat_id INTEGER PRIMARY KEY,
                        title TEXT,
                        last_message TEXT,
                        unread_count INTEGER DEFAULT 0,
                        last_updated INTEGER DEFAULT (strftime('%s', 'now'))
                    )"
                );
                
                // Messages table
                let _ = conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS messages (
                        message_id INTEGER PRIMARY KEY,
                        chat_id INTEGER,
                        text TEXT,
                        is_outgoing INTEGER,
                        timestamp INTEGER DEFAULT (strftime('%s', 'now'))
                    )"
                );
                
                MaxPlusStorage { connection: Some(conn) }
            }
            None => {
                // In-memory database for testing
                let conn = match Connection::open_in_memory() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Failed to create in-memory database: {}", e);
                        return MaxPlusStorage { connection: None };
                    }
                };
                
                // Initialize schema for in-memory
                let _ = conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS chats (
                        chat_id INTEGER PRIMARY KEY,
                        title TEXT,
                        last_message TEXT,
                        unread_count INTEGER DEFAULT 0,
                        last_updated INTEGER DEFAULT (strftime('%s', 'now'))
                    )"
                );
                
                // Messages table
                let _ = conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS messages (
                        message_id INTEGER PRIMARY KEY,
                        chat_id INTEGER,
                        text TEXT,
                        is_outgoing INTEGER,
                        timestamp INTEGER DEFAULT (strftime('%s', 'now'))
                    )"
                );
                
                MaxPlusStorage { connection: Some(conn) }
            }
        };
        
        connection
    }
    
    /// Get unread count from storage
    pub fn get_unread_count(&self) -> i32 {
        self.connection.as_ref()
            .and_then(|conn| {
                conn.query_row(
                    "SELECT COALESCE(SUM(unread_count), 0) FROM chats",
                    [],
                    |row| row.get::<_, i32>(0)
                ).ok()
            })
            .unwrap_or(0)
    }
    
    /// Increment unread count for a chat
    pub fn increment_unread(&self, chat_id: i64) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "INSERT INTO chats (chat_id, unread_count) VALUES (?1, 1) ON CONFLICT (chat_id) DO UPDATE SET unread_count = unread_count + 1",
                [chat_id],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }
    
    /// Set chat info (title, last message)
    pub fn set_chat_info(&self, chat_id: i64, title: &str, last_message: &str) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "INSERT INTO chats (chat_id, title, last_message, unread_count) VALUES (?1, ?2, ?3, 0) ON CONFLICT (chat_id) DO UPDATE SET title = ?2, last_message = ?3, last_updated = strftime('%s', 'now')",
                rusqlite::params![chat_id, title, last_message],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }
    
    /// Get all chats ordered by last updated
    pub fn get_chats(&self) -> anyhow::Result<Vec<(i64, String, String, i32, i64)>> {
        if let Some(conn) = &self.connection {
            let mut chats = Vec::new();
            
            let mut stmt = conn.prepare(
                "SELECT chat_id, title, last_message, unread_count, last_updated FROM chats ORDER BY last_updated DESC"
            )?;
            
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            
            for row_row in rows {
                if let Ok((id, title, msg, unread, updated)) = row_row {
                    chats.push((id, title, msg, unread, updated));
                }
            }
            
            Ok(chats)
        } else {
            Ok(vec![])
        }
    }
    
    /// Add a message to a chat
    pub fn add_message(&self, chat_id: i64, message_id: i64, text: &str, is_outgoing: bool) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "INSERT OR IGNORE INTO messages (chat_id, message_id, text, is_outgoing, timestamp) VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'))",
                rusqlite::params![chat_id, message_id, text, is_outgoing as i32],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }
    
    /// Get messages for a chat
    pub fn get_messages(&self, chat_id: i64, limit: usize) -> anyhow::Result<Vec<(i64, String, bool, i64)>> {
        if let Some(conn) = &self.connection {
            let mut messages = Vec::new();
            
            let mut stmt = conn.prepare(
                "SELECT message_id, text, is_outgoing, timestamp FROM messages WHERE chat_id = ?1 ORDER BY timestamp DESC LIMIT ?2"
            )?;
            
            let rows = stmt.query_map(rusqlite::params![chat_id, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)? != 0, // is_outgoing: true if non-zero
                    row.get::<_, i64>(3)?,
                ))
            })?;
            
            for row_row in rows {
                if let Ok((id, text, outgoing, timestamp)) = row_row {
                    messages.push((id, text, outgoing, timestamp));
                }
            }
            
            Ok(messages)
        } else {
            Ok(vec![])
        }
    }
}