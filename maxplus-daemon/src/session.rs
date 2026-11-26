//! Daemon session: authentication state, Max connection and local cache.

use crate::config::Config;
use crate::identity::generate_identity;
use crate::storage::MaxPlusStorage;
use anyhow::{anyhow, bail, Context, Result};
use rumax::models::Identity;
use rumax::MaxClient;
use serde_json::{json, Value};

/// Where the authentication flow currently stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPhase {
    Unauthenticated,
    CodeRequired,
    PasswordRequired,
    Authenticated,
}

impl AuthPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthPhase::Unauthenticated => "unauthenticated",
            AuthPhase::CodeRequired => "code_required",
            AuthPhase::PasswordRequired => "password_required",
            AuthPhase::Authenticated => "authenticated",
        }
    }
}

pub struct Session {
    pub client: MaxClient,
    pub storage: MaxPlusStorage,
    pub config: Config,
    identity: Identity,
    connected: bool,
    phase: AuthPhase,
    self_id: Option<i64>,
}

impl Session {
    pub fn new(storage: MaxPlusStorage) -> Self {
        let config = Config::load();
        let identity = config.identity.clone().unwrap_or_else(generate_identity);
        let phase = if config.is_authenticated() {
            AuthPhase::Authenticated
        } else if config.temp_token.is_some() {
            if config.password_track_id.is_some() {
                AuthPhase::PasswordRequired
            } else {
                AuthPhase::CodeRequired
            }
        } else {
            AuthPhase::Unauthenticated
        };

        Self {
            client: MaxClient::new(),
            storage,
            config,
            identity,
            connected: false,
            phase,
            self_id: None,
        }
    }

    pub fn phase(&self) -> AuthPhase {
        self.phase
    }

    pub fn status(&self) -> Value {
        json!({
            "ok": true,
            "authenticated": self.config.is_authenticated(),
            "connected": self.connected,
            "phone": self.config.phone,
            "user_id": self.self_id.or_else(|| self.config.username.as_ref().and_then(|s| s.parse::<i64>().ok())),
            "state": self.phase.as_str(),
            "hint": self.config.password_hint,
        })
    }

    /// Reconnect using the persisted identity and clear any previous session.
    async fn reconnect(&mut self, fresh: bool) -> Result<()> {
        if self.connected {
            self.client.disconnect().await;
            self.connected = false;
        }
        if fresh {
            self.client = MaxClient::new();
        }
        let identity = self.identity.clone();
        self.client
            .connect(identity, true)
            .await
            .context("Cannot connect to Max API")?;
        self.connected = true;
        Ok(())
    }

    /// Mark connection as dropped (e.g. ping failure detected by client call).
    pub fn note_disconnected(&mut self) {
        self.connected = false;
    }

    /// Restore a saved LOGIN token and refill the local message cache.
    /// Failure here is non-fatal: cached data remains available offline.
    pub async fn restore(&mut self) -> Result<()> {
        if !self.config.is_authenticated() {
            return Ok(());
        }
        self.reconnect(false).await?;
        let res: Result<()> = async {
            let token = self
                .config
                .token
                .clone()
                .ok_or_else(|| anyhow!("No saved token"))?;
            self.client.set_token(token).await;
            let sync_state = self.config.sync_state.clone().unwrap_or_default();
            let (first, second, new_state) = self
                .client
                .sync(Some(sync_state))
                .await
                .context("Session sync failed")?;

            self.sync_chats_from(&first);
            if let Some(second) = &second {
                self.sync_chats_from(second);
            }
            self.update_user_id(&first, &second);

            if let Some(token) = self.client.get_token().await {
                self.config.token = Some(token);
            }
            self.config.sync_state = Some(new_state);
            self.config.last_login = Some(now_secs());
            self.config.save()?;
            Ok(())
        }
        .await;
        if res.is_err() {
            self.connected = false;
        }
        res
    }

    /// Step 1 of login: request an SMS code for `phone`.
    pub async fn auth_start(&mut self, phone: &str) -> Result<Value> {
        let phone = phone.trim();
        let digits = phone.chars().filter(|c| c.is_ascii_digit()).count();
        if digits < 10 {
            bail!("Invalid phone number: expected at least 10 digits");
        }

        self.identity = generate_identity();
        self.config.identity = Some(self.identity.clone());
        self.config.authenticated = false;
        self.config.token = None;
        self.config.temp_token = None;
        self.config.password_track_id = None;
        self.config.password_hint = None;
        self.config.phone = Some(phone.to_string());
        self.config.save()?;

        self.reconnect(true).await?;
        let response = self
            .client
            .start_auth(phone.to_string())
            .await
            .context("Cannot request SMS code")?;
        let temp = response
            .payload
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Auth response has no temporary token"))?;
        self.client.set_temp_token(temp.clone()).await;
        self.config.temp_token = Some(temp);
        self.config.save()?;

        self.phase = AuthPhase::CodeRequired;
        Ok(json!({ "ok": true, "state": self.phase.as_str() }))
    }

    /// Step 2 of login: verify the SMS code.
    pub async fn auth_code(&mut self, code: &str) -> Result<Value> {
        let code = code.trim();
        if code.is_empty() {
            bail!("Empty verification code");
        }
        let temp = self
            .config
            .temp_token
            .clone()
            .ok_or_else(|| anyhow!("No pending login. Run auth start first."))?;

        self.reconnect(false).await?;
        self.client.set_temp_token(temp).await;
        let response = self
            .client
            .check_code(code.to_string())
            .await
            .context("Cannot verify SMS code")?;

        if let Some(challenge) = response.payload.get("passwordChallenge") {
            let track_id = challenge
                .get("trackId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let hint = challenge
                .get("hint")
                .and_then(Value::as_str)
                .unwrap_or("cloud password")
                .to_string();
            self.config.password_track_id = Some(track_id);
            self.config.password_hint = Some(hint.clone());
            self.config.save()?;
            self.phase = AuthPhase::PasswordRequired;
            return Ok(json!({
                "ok": true,
                "state": self.phase.as_str(),
                "hint": hint,
            }));
        }

        let tokens = response.payload.get("tokenAttrs");
        if let Some(token) = login_token(tokens) {
            self.client.set_token(token).await;
            return self.finish_login().await;
        }
        if let Some(register) = tokens
            .and_then(|t| t.get("REGISTER"))
            .and_then(|r| r.get("token"))
            .and_then(Value::as_str)
        {
            self.config.temp_token = Some(register.to_string());
            self.config.save()?;
            bail!("Account is not registered. Use the rumax register example first.");
        }
        bail!("Verification did not return a LOGIN token");
    }

    /// Step 3 of login (optional): cloud password from `passwordChallenge`.
    pub async fn auth_password(&mut self, password: &str) -> Result<Value> {
        if password.is_empty() {
            bail!("Empty password");
        }
        let track_id = self
            .config
            .password_track_id
            .clone()
            .ok_or_else(|| anyhow!("No password challenge is pending"))?;
        let temp = self.config.temp_token.clone().unwrap_or_default();
        self.reconnect(false).await?;
        if !temp.is_empty() {
            self.client.set_temp_token(temp).await;
        }
        let response = self
            .client
            .check_password(password.to_string(), track_id)
            .await
            .context("Cannot verify cloud password")?;
        if login_token(response.payload.get("tokenAttrs")).is_none() {
            bail!("Cloud password was rejected");
        }
        self.finish_login().await
    }

    async fn finish_login(&mut self) -> Result<Value> {
        let token = self
            .client
            .get_token()
            .await
            .ok_or_else(|| anyhow!("No LOGIN token received"))?;
        self.config.token = Some(token);
        self.config.authenticated = true;
        self.config.temp_token = None;
        self.config.password_track_id = None;
        self.config.password_hint = None;
        self.config.last_login = Some(now_secs());
        self.config.save()?;
        self.phase = AuthPhase::Authenticated;

        // Sync is best-effort; the token is already persisted.
        if let Err(err) = self.restore().await {
            eprintln!("Warning: initial sync failed: {}", err);
        }
        Ok(json!({ "ok": true, "state": self.phase.as_str() }))
    }

    pub async fn logout(&mut self) -> Result<Value> {
        self.config.authenticated = false;
        self.config.token = None;
        self.config.temp_token = None;
        self.config.password_track_id = None;
        self.config.password_hint = None;
        self.config.save()?;
        if self.connected {
            self.client.disconnect().await;
            self.connected = false;
        }
        self.phase = AuthPhase::Unauthenticated;
        Ok(json!({ "ok": true, "state": self.phase.as_str() }))
    }

    /// Cached chats. Never invent data when there is no real session.
    pub fn chats(&self) -> Value {
        let chats = self
            .storage
            .get_chats()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, title, last, unread, _updated, avatar)| {
                json!({
                    "id": id,
                    "title": title,
                    "last_message": last,
                    "unread": unread,
                    "avatar": avatar,
                })
            })
            .collect::<Vec<_>>();
        json!({ "ok": true, "chats": chats })
    }

    pub fn messages(&self, chat_id: i64, limit: usize) -> Value {
        let messages = self
            .storage
            .get_messages(chat_id, limit)
            .unwrap_or_default()
            .into_iter()
            .map(|(id, text, outgoing, ts, attaches_json, status)| {
                let attaches: Value = serde_json::from_str(&attaches_json).unwrap_or_else(|_| json!([]));
                json!({
                    "id": id,
                    "text": text,
                    "outgoing": outgoing,
                    "timestamp": ts,
                    "attaches": attaches,
                    "status": status,
                })
            })
            .collect::<Vec<_>>();
        json!({ "ok": true, "messages": messages })
    }

    /// Send via the live Max connection and mirror the result into local cache.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<Value> {
        let text = text.trim();
        if text.is_empty() {
            bail!("Empty message");
        }
        if !self.config.is_authenticated() || !self.connected {
            bail!("Daemon is not connected to Max. Check status first.");
        }
        self.client
            .send_message(chat_id, text.to_string(), None)
            .await
            .context("Cannot send message")?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let id = -millis;
        self.storage.add_message(chat_id, id, text, true, 0, "[]", 1)?;
        Ok(json!({ "ok": true, "id": id }))
    }

    fn update_user_id(&mut self, first: &rumax::models::Response, second: &Option<rumax::models::Response>) {
        let profile = second
            .as_ref()
            .and_then(|r| r.payload.get("profile"))
            .or_else(|| first.payload.get("profile"));
        if let Some(id) = profile
            .and_then(|p| p.get("contact"))
            .and_then(|c| c.get("id"))
            .and_then(Value::as_i64)
        {
            self.self_id = Some(id);
            let client = self.client.clone();
            tokio::spawn(async move { client.set_user_id(id as u64).await });
        }
    }

    /// Pull chat titles and last messages out of a sync response.
    fn sync_chats_from(&mut self, response: &rumax::models::Response) {
        let contact_map = collect_contact_names(&response.payload);
        let self_id = extract_self_id(&response.payload).or(self.self_id);
        if let Some(id) = self_id { self.self_id = Some(id); }
        let mut found = Vec::new();
        collect_chats(&response.payload, 0, &mut found);
        for (id, mut title, last, unread, mut avatar, ts) in found {
            if title == "Untitled" || title.is_empty() || avatar.is_empty() {
                if let Some((resolved_title, resolved_avatar)) = resolve_dialog_contact(&response.payload, id, self_id, &contact_map) {
                    if title == "Untitled" || title.is_empty() { title = resolved_title; }
                    if avatar.is_empty() { avatar = resolved_avatar; }
                }
            }
            if let Err(err) = self.storage.set_chat_info(id, &title, &last, &avatar, ts) {
                eprintln!("Cannot cache chat {}: {}", id, err);
                continue;
            }
            let _ = self.storage.set_unread_count(id, unread);
        }
        // Also store last messages into messages table when possible
        store_last_messages(&self.storage, &response.payload, self_id);
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn login_token(tokens: Option<&Value>) -> Option<String> {
    tokens
        .and_then(|t| t.get("LOGIN"))
        .and_then(|l| l.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_self_id(payload: &Value) -> Option<i64> {
    payload.get("profile")
        .and_then(|p| p.get("contact"))
        .and_then(|c| c.get("id"))
        .and_then(Value::as_i64)
        .or_else(|| payload.get("userId").and_then(Value::as_i64))
}

fn collect_contact_names(payload: &Value) -> std::collections::HashMap<i64, (String, String)> {
    let mut map = std::collections::HashMap::new();
    fn walk(v: &Value, map: &mut std::collections::HashMap<i64, (String, String)>) {
        match v {
            Value::Object(m) => {
                if let Some(contacts) = m.get("contacts").and_then(Value::as_array) {
                    for c in contacts {
                        if let Some(id) = c.get("id").and_then(Value::as_i64) {
                            let name = contact_display_name(c);
                            let avatar = c.get("baseUrl").or_else(|| c.get("baseRawUrl")).and_then(Value::as_str).unwrap_or("").to_string();
                            if !name.is_empty() {
                                map.insert(id, (name, avatar));
                            }
                        }
                    }
                }
                for child in m.values() { walk(child, map); }
            }
            Value::Array(arr) => { for child in arr { walk(child, map); } }
            _ => {}
        }
    }
    walk(payload, &mut map);
    map
}

fn contact_display_name(c: &Value) -> String {
    if let Some(names) = c.get("names").and_then(Value::as_array) {
        for n in names {
            let name = n.get("name").and_then(Value::as_str).unwrap_or("").trim();
            if !name.is_empty() { return name.to_string(); }
            let first = n.get("firstName").and_then(Value::as_str).unwrap_or("").trim();
            let last = n.get("lastName").and_then(Value::as_str).unwrap_or("").trim();
            let full = format!("{} {}", first, last).trim().to_string();
            if !full.is_empty() { return full; }
        }
    }
    c.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn resolve_dialog_contact(payload: &Value, chat_id: i64, self_id: Option<i64>, contacts: &std::collections::HashMap<i64, (String, String)>) -> Option<(String, String)> {
    fn find_chat<'a>(v: &'a Value, chat_id: i64, depth: u8) -> Option<&'a Value> {
        if depth > 6 { return None; }
        match v {
            Value::Object(m) => {
                if let Some(chats) = m.get("chats").and_then(Value::as_array) {
                    for c in chats {
                        if c.get("id").and_then(Value::as_i64) == Some(chat_id) { return Some(c); }
                    }
                }
                for child in m.values() { if let Some(f) = find_chat(child, chat_id, depth+1) { return Some(f); } }
            }
            Value::Array(arr) => { for child in arr { if let Some(f) = find_chat(child, chat_id, depth+1) { return Some(f); } } }
            _ => {}
        }
        None
    }
    let chat = find_chat(payload, chat_id, 0)?;
    if chat.get("type").and_then(Value::as_str) != Some("DIALOG") { return None; }
    let participants = chat.get("participants").and_then(Value::as_object)?;
    for k in participants.keys() {
        if let Ok(pid) = k.parse::<i64>() {
            if Some(pid) == self_id { continue; }
            if let Some(info) = contacts.get(&pid) { return Some(info.clone()); }
        }
    }
    for k in participants.keys() {
        if let Ok(pid) = k.parse::<i64>() {
            if Some(pid) != self_id {
                if let Some(info) = contacts.get(&pid) { return Some(info.clone()); }
            }
        }
    }
    None
}

fn store_last_messages(storage: &MaxPlusStorage, payload: &Value, self_id: Option<i64>) {
    fn walk(v: &Value, out: &mut Vec<(i64, i64, String, bool, i64, String)>, self_id: Option<i64>, depth: u8) {
        if depth > 6 { return; }
        match v {
            Value::Object(m) => {
                if let Some(chats) = m.get("chats").and_then(Value::as_array) {
                    for c in chats {
                        let chat_id = c.get("id").and_then(Value::as_i64).unwrap_or(0);
                        if let Some(lm) = c.get("lastMessage") {
                            if let Some(msg_id) = lm.get("id").and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())) {
                                let text = extract_msg_text(lm);
                                let ts = lm.get("time").and_then(Value::as_i64).unwrap_or(0);
                                let sender = lm.get("sender").and_then(Value::as_i64).unwrap_or(0);
                                let outgoing = self_id.map(|me| me == sender).unwrap_or(false);
                                let atts = extract_attaches(lm);
                                out.push((chat_id, msg_id, text, outgoing, ts, atts));
                            }
                        }
                    }
                }
                for child in m.values() { walk(child, out, self_id, depth+1); }
            }
            Value::Array(arr) => { for child in arr { walk(child, out, self_id, depth+1); } }
            _ => {}
        }
    }
    let mut msgs = Vec::new();
    walk(payload, &mut msgs, self_id, 0);
    for (chat_id, msg_id, text, outgoing, ts, atts) in msgs {
        let ts_sec = if ts > 1_000_000_000_000 { ts / 1000 } else { ts };
        let status = if outgoing { 1 } else { 0 }; let _ = storage.add_message(chat_id, msg_id, &text, outgoing, ts_sec, &atts, status);
    }
}



fn extract_attaches(msg: &Value) -> String {
    let mut out = Vec::new();
    if let Some(attaches) = msg.get("attaches").and_then(Value::as_array) {
        for a in attaches {
            let tp = a.get("_type").and_then(Value::as_str).unwrap_or("");
            match tp {
                "PHOTO" => {
                    let url = a.get("baseUrl")
                        .or_else(|| a.get("url"))
                        .or_else(|| a.get("previewUrl"))
                        .and_then(Value::as_str).unwrap_or("");
                    if !url.is_empty() {
                        out.push(json!({"type":"photo","url":url}));
                    }
                }
                "STICKER" => {
                    let url = a.get("url")
                        .or_else(|| a.get("previewUrl"))
                        .or_else(|| a.get("baseUrl"))
                        .or_else(|| a.get("lottieUrl"))
                        .and_then(Value::as_str).unwrap_or("");
                    let sticker_id = a.get("stickerId").and_then(Value::as_i64).unwrap_or(0);
                    if !url.is_empty() || sticker_id != 0 {
                        out.push(json!({"type":"sticker","url":url,"stickerId":sticker_id}));
                    }
                }
                "FILE" => {
                    let url = a.get("url")
                        .or_else(|| a.get("fileUrl"))
                        .and_then(Value::as_str).unwrap_or("");
                    let name = a.get("fileName")
                        .or_else(|| a.get("name"))
                        .and_then(Value::as_str).unwrap_or("Файл");
                    let size = a.get("size").and_then(Value::as_i64).unwrap_or(0);
                    out.push(json!({"type":"file","url":url,"name":name,"size":size}));
                }
                "VIDEO" => {
                    let url = a.get("url")
                        .or_else(|| a.get("baseUrl"))
                        .and_then(Value::as_str).unwrap_or("");
                    let thumb = a.get("thumbnail")
                        .or_else(|| a.get("previewUrl"))
                        .or_else(|| a.get("baseUrl"))
                        .and_then(Value::as_str).unwrap_or("");
                    let duration = a.get("duration").and_then(Value::as_i64).unwrap_or(0);
                    out.push(json!({"type":"video","url":url,"thumbnail":thumb,"duration":duration}));
                }
                _ => {}
            }
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string())
}
fn extract_msg_text(msg: &Value) -> String {
    if let Some(t) = msg.get("text").and_then(Value::as_str) {
        let t = t.trim();
        if !t.is_empty() { return t.to_string(); }
    }
    if let Some(attaches) = msg.get("attaches").and_then(Value::as_array) {
        for a in attaches {
            let tp = a.get("_type").and_then(Value::as_str).unwrap_or("");
            match tp {
                "PHOTO" => return "[Фото]".to_string(),
                "VIDEO" => return "[Видео]".to_string(),
                "FILE" => return "[Файл]".to_string(),
                "AUDIO" => return "[Аудио]".to_string(),
                "STICKER" => return "[Стикер]".to_string(),
                "CONTROL" => {
                    if let Some(t) = a.get("title").and_then(Value::as_str) { return t.to_string(); }
                    return "[Системное сообщение]".to_string();
                }
                _ => {}
            }
        }
    }
    String::new()
}

/// The Max sync payload nests chats in several places; walk it defensively.
fn collect_chats(value: &Value, depth: u8, out: &mut Vec<(i64, String, String, i32, String, i64)>) {
    if depth > 6 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(items) = map.get("chats").and_then(Value::as_array) {
                for item in items {
                    if let Some(row) = parse_chat(item) {
                        out.push(row);
                    }
                }
            }
            for v in map.values() {
                collect_chats(v, depth + 1, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_chats(v, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn parse_chat(item: &Value) -> Option<(i64, String, String, i32, String, i64)> {
    let id = item
        .get("id")
        .or_else(|| item.get("chatId"))
        .and_then(Value::as_i64)?;
    let title = item
        .get("title")
        .or_else(|| item.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_string();
    let last_msg_val = item.get("lastMessage").or_else(|| item.get("last_message")).cloned().unwrap_or(Value::Null);
    let last = extract_msg_text(&last_msg_val);
    let unread = item
        .get("newMessages")
        .or_else(|| item.get("unreadCount"))
        .or_else(|| item.get("unread"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as i32;
    let avatar = item
        .get("baseIconUrl")
        .or_else(|| item.get("baseRawIconUrl"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut ts = last_msg_val.get("time").and_then(Value::as_i64)
        .or_else(|| item.get("lastEventTime").and_then(Value::as_i64))
        .or_else(|| item.get("modified").and_then(Value::as_i64))
        .unwrap_or(0);
    if ts > 10_000_000_000 { ts /= 1000; } // ms -> s
    Some((id, title, last, unread, avatar, ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chats_from_nested_sync_payload() {
        let payload = json!({
            "chats": [{
                "chats": [
                    {"id": 42, "title": "Test", "lastMessage": "hi", "unreadCount": 2}
                ]
            }]
        });
        let mut out = Vec::new();
        collect_chats(&payload, 0, &mut out);
        assert_eq!(out, vec![(42, "Test".into(), "hi".into(), 2, "".into())]);
    }

    #[test]
    fn login_token_is_extracted_from_token_attrs() {
        let attrs = json!({"tokenAttrs": {"LOGIN": {"token": "abc"}}});
        assert_eq!(login_token(attrs.get("tokenAttrs")).as_deref(), Some("abc"));
    }

    #[test]
    fn empty_phone_is_rejected() {
        let mut session = Session::new(MaxPlusStorage::new(&None));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(rt.block_on(session.auth_start("abc")).is_err());
    }
}
