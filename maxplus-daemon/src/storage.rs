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
                if let Some(parent) = Path::new(path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match Connection::open(path) {
                    Ok(c) => {
                        let s = MaxPlusStorage { connection: Some(c) };
                        s.init_schema();
                        s
                    }
                    Err(e) => {
                        eprintln!("Failed to open database at {:?}: {}", path, e);
                        MaxPlusStorage { connection: None }
                    }
                }
            }
            None => match Connection::open_in_memory() {
                Ok(c) => {
                    let s = MaxPlusStorage { connection: Some(c) };
                    s.init_schema();
                    s
                }
                Err(e) => {
                    eprintln!("Failed to create in-memory database: {}", e);
                    MaxPlusStorage { connection: None }
                }
            },
        };
        connection
    }

    fn init_schema(&self) {
        let Some(conn) = &self.connection else { return };
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chats (
                chat_id INTEGER PRIMARY KEY,
                title TEXT,
                last_message TEXT,
                unread_count INTEGER DEFAULT 0,
                last_updated INTEGER DEFAULT (strftime('%s', 'now')),
                avatar_url TEXT DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS messages (
                message_id INTEGER PRIMARY KEY,
                chat_id INTEGER,
                text TEXT,
                is_outgoing INTEGER,
                timestamp INTEGER DEFAULT (strftime('%s', 'now')),
                attaches_json TEXT DEFAULT '[]',
                status INTEGER DEFAULT 0
            );",
        );
        // Migration for existing DBs
        let _ = conn.execute_batch("ALTER TABLE chats ADD COLUMN avatar_url TEXT DEFAULT '';");
        let _ = conn.execute_batch("ALTER TABLE messages ADD COLUMN attaches_json TEXT DEFAULT '[]';");
        let _ = conn.execute_batch("ALTER TABLE messages ADD COLUMN status INTEGER DEFAULT 0;");
    }

    pub fn get_unread_count(&self) -> i32 {
        self.connection
            .as_ref()
            .and_then(|conn| {
                conn.query_row("SELECT COALESCE(SUM(unread_count), 0) FROM chats", [], |row| {
                    row.get::<_, i32>(0)
                })
                .ok()
            })
            .unwrap_or(0)
    }

    pub fn increment_unread(&self, chat_id: i64) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "INSERT INTO chats (chat_id, unread_count) VALUES (?1, 1) ON CONFLICT (chat_id) DO UPDATE SET unread_count = unread_count + 1",
                [chat_id],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }

    pub fn set_unread_count(&self, chat_id: i64, count: i32) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "UPDATE chats SET unread_count = ?2 WHERE chat_id = ?1",
                rusqlite::params![chat_id, count.max(0)],
            )
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }

    /// Set chat info (title, last message, avatar). last_updated<=0 means now.
    pub fn set_chat_info(&self, chat_id: i64, title: &str, last_message: &str, avatar_url: &str, last_updated: i64) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            conn.execute(
                "INSERT INTO chats (chat_id, title, last_message, unread_count, avatar_url, last_updated)
                 VALUES (?1, ?2, ?3, 0, ?4, CASE WHEN ?5 > 0 THEN ?5 ELSE strftime('%s','now') END)
                 ON CONFLICT (chat_id) DO UPDATE SET
                   title = CASE WHEN ?2 != '' AND ?2 != 'Untitled' THEN ?2 ELSE title END,
                   last_message = CASE WHEN ?3 != '' THEN ?3 ELSE last_message END,
                   avatar_url = CASE WHEN ?4 != '' THEN ?4 ELSE avatar_url END,
                   last_updated = CASE WHEN ?5 > 0 THEN MAX(?5, COALESCE(last_updated,0)) ELSE strftime('%s','now') END",
                rusqlite::params![chat_id, title, last_message, avatar_url, last_updated],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }

    /// Get all chats ordered by last updated. Returns (id, title, last_message, unread, updated, avatar_url).
    pub fn get_chats(&self) -> anyhow::Result<Vec<(i64, String, String, i32, i64, String)>> {
        if let Some(conn) = &self.connection {
            let mut stmt = conn.prepare(
                "SELECT chat_id, COALESCE(title,''), COALESCE(last_message,''), COALESCE(unread_count,0), COALESCE(last_updated,0), COALESCE(avatar_url,'')
                 FROM chats ORDER BY last_updated DESC"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?;
            let mut chats = Vec::new();
            for r in rows {
                if let Ok(v) = r { chats.push(v); }
            }
            Ok(chats)
        } else {
            Ok(vec![])
        }
    }

    pub fn add_message(&self, chat_id: i64, message_id: i64, text: &str, is_outgoing: bool, timestamp: i64, attaches_json: &str, status: i32) -> anyhow::Result<()> {
        if let Some(conn) = &self.connection {
            let atts = if attaches_json.is_empty() { "[]" } else { attaches_json };
            conn.execute(
                "INSERT INTO messages (chat_id, message_id, text, is_outgoing, timestamp, attaches_json, status) VALUES (?1, ?2, ?3, ?4, CASE WHEN ?5>0 THEN ?5 ELSE strftime('%s','now') END, ?6, ?7)
                 ON CONFLICT(message_id) DO UPDATE SET
                   text=excluded.text,
                   is_outgoing=excluded.is_outgoing,
                   timestamp=CASE WHEN ?5>0 THEN ?5 ELSE messages.timestamp END,
                   attaches_json=CASE WHEN ?6!='[]' THEN ?6 ELSE messages.attaches_json END,
                   status=CASE WHEN ?7>COALESCE(messages.status,0) THEN ?7 ELSE COALESCE(messages.status,0) END",
                rusqlite::params![chat_id, message_id, text, is_outgoing as i32, timestamp, atts, status],
            ).map_err(|e| anyhow::anyhow!("DB error: {}", e))?;
        }
        Ok(())
    }

    pub fn get_messages(&self, chat_id: i64, limit: usize) -> anyhow::Result<Vec<(i64, String, bool, i64, String, i32)>> {
        if let Some(conn) = &self.connection {
            let mut stmt = conn.prepare(
                "SELECT message_id, COALESCE(text,''), is_outgoing, COALESCE(timestamp,0), COALESCE(attaches_json,'[]'), COALESCE(status,0) FROM messages WHERE chat_id = ?1 ORDER BY timestamp DESC, message_id DESC LIMIT ?2"
            )?;
            let rows = stmt.query_map(rusqlite::params![chat_id, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)? != 0,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i32>(5)?,
                ))
            })?;
            let mut messages = Vec::new();
            for r in rows {
                if let Ok(v) = r { messages.push(v); }
            }
            Ok(messages)
        } else {
            Ok(vec![])
        }
    }
}
