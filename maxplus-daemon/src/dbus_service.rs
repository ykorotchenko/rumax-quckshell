use crate::session::Session;
use anyhow::{anyhow, Context, Result};
use dbus::tree::{Factory, MTFn, MethodErr, MethodInfo, MethodResult};
use dbus::{BusType, Connection, NameFlag};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex};
use tokio::runtime::Handle;

type Info<'a> = &'a MethodInfo<'a, MTFn<()>, ()>;

pub const SERVICE: &str = "org.maxplus.Client";
pub const PATH: &str = "/org/maxplus/Client";
pub const INTERFACE: &str = "org.maxplus.Client";

pub fn start(state: Arc<Mutex<Session>>, handle: Handle) -> Result<()> {
    start_with_state(state, handle)
}

pub fn start_with_db_path(db_path: String) -> Result<()> {
    // Fallback: no session; create a dummy session-less service not supported here.
    // Keep for compatibility by opening storage directly through a minimal session.
    let storage = crate::storage::MaxPlusStorage::new(&Some(db_path));
    let session = Session::new(storage);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let handle = rt.handle().clone();
    let state = Arc::new(Mutex::new(session));
    // Leak rt to keep handle alive - fallback path only.
    std::mem::forget(rt);
    start_with_state(state, handle)
}

type TypingState = Arc<Mutex<std::collections::HashMap<i64, i64>>>; // chat_id -> expires_unix_secs

fn start_with_state(state: Arc<Mutex<Session>>, handle: Handle) -> Result<()> {
    let typing: TypingState = Arc::new(Mutex::new(std::collections::HashMap::new()));

    // Listen for typing events (opcode 129), re-subscribe if client replaced
    {
        let state2 = state.clone();
        let typing2 = typing.clone();
        handle.spawn(async move {
            loop {
                let mut rx = { state2.lock().unwrap().client.subscribe() };
                loop {
                    match rx.recv().await {
                        Ok(val) => {
                            if val.get("type").and_then(|v| v.as_str()) == Some("rx") {
                                if let Some(resp) = val.get("response") {
                                    let opcode = resp.get("opcode").and_then(|v| v.as_u64()).unwrap_or(0);
                                    if opcode == 129 {
                                        let payload = resp.get("payload").cloned().unwrap_or(Value::Null);
                                        let chat_id = payload.get("chatId").and_then(|v| v.as_i64())
                                            .or_else(|| payload.get("chat_id").and_then(|v| v.as_i64()))
                                            .or_else(|| payload.get("cid").and_then(|v| v.as_i64()))
                                            .unwrap_or(0);
                                        eprintln!("Max+ typing event: chat_id={} payload={}", chat_id, payload);
                                        if chat_id != 0 {
                                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                                            typing2.lock().unwrap().insert(chat_id, now + 6);
                                        }
                                    }
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        });
    }

    let (tx, rx) = mpsc::channel::<Result<()>>();
    std::thread::Builder::new()
        .name("maxplus-dbus".into())
        .spawn(move || {
            let c = match Connection::get_private(BusType::Session) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(anyhow!("No session bus: {}", e)));
                    return;
                }
            };

            let ready = match c.register_name(SERVICE, NameFlag::ReplaceExisting as u32) {
                Ok(dbus::RequestNameReply::PrimaryOwner) => Ok(()),
                Ok(other) => Err(anyhow!("Cannot own {}: {:?}", SERVICE, other)),
                Err(e) => Err(anyhow!("Cannot own {}: {}", SERVICE, e)),
            };
            let ok = ready.is_ok();
            let _ = tx.send(ready);
            if !ok {
                return;
            }

            let f = Factory::new_fn::<()>();
            let tree = f.tree(()).add(
                f.object_path(PATH, ())
                    .introspectable()
                    .add(build_interface(&f, state, handle, typing)),
            );
            if let Err(e) = tree.set_registered(&c, true) {
                eprintln!("D-Bus registration failed: {}", e);
                return;
            }
            for _ in tree.run(&c, c.iter(200)) {}
        })
        .context("Cannot spawn D-Bus thread")?;

    rx.recv().context("D-Bus thread stopped during setup")?
}


fn reply(m: Info, val: Value) -> MethodResult {
    Ok(vec![m.msg.method_return().append1(val.to_string())])
}

fn extract_text(msg: &Value) -> String {
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

fn extract_attaches_json(msg: &Value) -> String {
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
fn build_interface(f: &Factory<MTFn<()>, ()>, state: Arc<Mutex<Session>>, handle: Handle, typing: TypingState) -> dbus::tree::Interface<MTFn<()>, ()> {
    let mut interface = f.interface(INTERFACE, ());

    interface = interface.add_m(
        f.method("Ping", (), move |m| {
            reply(m, json!({"ok": true, "service": "maxplus", "version": env!("CARGO_PKG_VERSION")}))
        })
        .outarg::<&str, _>("reply"),
    );

    {
        let state = state.clone();
        interface = interface.add_m(
            f.method("Status", (), move |m| {
                let guard = state.lock().unwrap();
                let unread = guard.storage.get_unread_count();
                let chats = guard.storage.get_chats().unwrap_or_default();
                reply(m, json!({
                    "ok": true,
                    "connected": true,
                    "authenticated": guard.status()["authenticated"].as_bool().unwrap_or(false),
                    "unread": unread,
                    "chats": chats.len()
                }))
            })
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        interface = interface.add_m(
            f.method("GetChats", (), move |m| {
                let guard = state.lock().unwrap();
                let chats = guard.storage.get_chats().unwrap_or_default();
                let items: Vec<Value> = chats.into_iter().enumerate().map(|(i, (id, title, last_message, unread, updated, avatar))| {
                    json!({
                        "id": id,
                        "title": title,
                        "last_message": last_message,
                        "unread": unread,
                        "updated": updated,
                        "avatar": avatar,
                        "online": i % 3 != 0
                    })
                }).collect();
                reply(m, json!({"ok": true, "chats": items}))
            })
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        let handle = handle.clone();
        interface = interface.add_m(
            f.method("GetMessages", (), move |m| {
                let (chat_id, limit) = m.msg.read2::<i64, i32>().map_err(MethodErr::from)?;
                let limit = (limit.max(1) as usize).min(500);

                let fetch_and_store = |st: Arc<Mutex<Session>>, chat_id: i64, limit: usize| async move {
                    let client = { st.lock().unwrap().client.clone() };
                    let opts = rumax::models::FetchHistoryOptions {
                        backward: limit as i64,
                        forward: 0,
                        from_time: None,
                        ..Default::default()
                    };
                    if let Ok(resp) = client.fetch_history(chat_id, Some(opts)).await {
                        if let Some(msgs) = resp.payload.get("messages").and_then(|v| v.as_array()) {
                            let guard = st.lock().unwrap();
                            let self_id = guard.status().get("user_id").and_then(|v| v.as_i64()).unwrap_or(0);
                            for msg in msgs {
                                let mid = msg.get("id").and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
                                let text = extract_text(msg);
                                let sender = msg.get("sender").and_then(Value::as_i64).unwrap_or(0);
                                let outgoing = self_id != 0 && sender == self_id;
                                let ts = msg.get("time").and_then(Value::as_i64).unwrap_or(0);
                                let ts_sec = if ts > 1_000_000_000_000 { ts / 1000 } else { ts };
                                let atts = extract_attaches_json(msg);
                                let status = if outgoing { 1 } else { 0 };
                                let _ = guard.storage.add_message(chat_id, mid, &text, outgoing, ts_sec, &atts, status);
                            }
                        }
                    }
                };

                let has_cached = {
                    let guard = state.lock().unwrap();
                    guard.storage.get_messages(chat_id, 1).map(|v| !v.is_empty()).unwrap_or(false)
                };

                if has_cached {
                    let st = state.clone();
                    handle.spawn(fetch_and_store(st, chat_id, limit));
                } else {
                    let st = state.clone();
                    let _ = handle.block_on(fetch_and_store(st, chat_id, limit));
                }

                let guard = state.lock().unwrap();
                let mut msgs = guard.storage.get_messages(chat_id, limit).unwrap_or_default();
                msgs.reverse();
                let items: Vec<Value> = msgs.into_iter().map(|(id, text, outgoing, ts, attaches_json, status)| {
                    let attaches: Value = serde_json::from_str(&attaches_json).unwrap_or_else(|_| json!([]));
                    json!({"id": id, "text": text, "outgoing": outgoing, "timestamp": ts, "attaches": attaches, "status": status})
                }).collect();
                reply(m, json!({"ok": true, "chat_id": chat_id, "messages": items}))
            })
            .inarg::<i64, _>("chat_id")
            .inarg::<i32, _>("limit")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        let handle = handle.clone();
        interface = interface.add_m(
            f.method("SendMessage", (), move |m| {
                let (chat_id, text) = m.msg.read2::<i64, &str>().map_err(MethodErr::from)?;
                let text = text.trim().to_string();
                if text.is_empty() {
                    return reply(m, json!({"ok": false, "error": "Empty message"}));
                }
                let (client, authed) = {
                    let s = state.lock().unwrap();
                    let authed = s.status()["authenticated"].as_bool().unwrap_or(false) && s.status()["connected"].as_bool().unwrap_or(false);
                    (s.client.clone(), authed)
                };
                if !authed {
                    return reply(m, json!({"ok": false, "error": "Not connected"}));
                }
                let send_res = handle.block_on(async move {
                    let resp = client.send_message(chat_id, text.clone(), None).await.map_err(|e| e.to_string())?;
                    let p = &resp.payload;
                    let real_id = p.get("message").and_then(|m| m.get("id"))
                        .or_else(|| p.get("messages").and_then(|a| a.get(0)).and_then(|m| m.get("id")))
                        .or_else(|| p.get("chat").and_then(|c| c.get("lastMessage")).and_then(|m| m.get("id")))
                        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
                        .unwrap_or(0);
                    Ok::<_, String>((text, real_id))
                });
                match send_res {
                    Ok((sent_text, real_id)) => {
                        let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                        let id = if real_id != 0 { real_id } else { -millis };
                        let guard = state.lock().unwrap();
                        let _ = guard.storage.add_message(chat_id, id, &sent_text, true, 0, "[]", 1);
                        let chats = guard.storage.get_chats().unwrap_or_default();
                        let title = chats.iter().find(|(cid,_,_,_,_,_)| *cid==chat_id).map(|(_,t,_,_,_,_)| t.clone()).unwrap_or_else(|| format!("Chat {chat_id}"));
                        let _ = guard.storage.set_chat_info(chat_id, &title, &sent_text, "", 0);
                        reply(m, json!({"ok": true, "chat_id": chat_id, "message_id": id}))
                    }
                    Err(e) => {
                        let es = e.to_string();
                        if es.contains("Not connected") || es.contains("Connection closed") || es.contains("ping") {
                            let mut guard = state.lock().unwrap();
                            guard.note_disconnected();
                        }
                        reply(m, json!({"ok": false, "error": es}))
                    }
                }
            })
            .inarg::<i64, _>("chat_id")
            .inarg::<&str, _>("text")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        interface = interface.add_m(
            f.method("GetUnreadCount", (), move |m| {
                let guard = state.lock().unwrap();
                reply(m, json!({"ok": true, "unread": guard.storage.get_unread_count()}))
            })
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        interface = interface.add_m(
            f.method("MarkRead", (), move |m| {
                let chat_id = m.msg.read1::<i64>().map_err(MethodErr::from)?;
                let guard = state.lock().unwrap();
                let _ = guard.storage.set_unread_count(chat_id, 0);
                reply(m, json!({"ok": true, "chat_id": chat_id}))
            })
            .inarg::<i64, _>("chat_id")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let typing = typing.clone();
        interface = interface.add_m(
            f.method("GetTyping", (), move |m| {
                let chat_id = m.msg.read1::<i64>().map_err(MethodErr::from)?;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                let mut map = typing.lock().unwrap();
                // cleanup expired
                map.retain(|_, exp| *exp > now);
                let is_typing = map.get(&chat_id).map(|exp| *exp > now).unwrap_or(false);
                reply(m, json!({"ok": true, "chat_id": chat_id, "typing": is_typing}))
            })
            .inarg::<i64, _>("chat_id")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        let handle = handle.clone();
        interface = interface.add_m(
            f.method("SendTyping", (), move |m| {
                let chat_id = m.msg.read1::<i64>().map_err(MethodErr::from)?;
                let (client, authed) = {
                    let s = state.lock().unwrap();
                    let authed = s.status()["authenticated"].as_bool().unwrap_or(false)
                        && s.status()["connected"].as_bool().unwrap_or(false);
                    (s.client.clone(), authed)
                };
                if !authed {
                    return reply(m, json!({"ok": false, "error": "Not connected"}));
                }
                let send_res = handle.block_on(async move {
                    // Max protocol: opcode 65 is typing notification
                    client
                        .send_and_wait(65, serde_json::json!({"chatId": chat_id, "type": "TYPING"}), 0)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
                match send_res {
                    Ok(()) => reply(m, json!({"ok": true})),
                    Err(e) => {
                        if e.contains("Not connected") || e.contains("Connection closed") || e.contains("ping") {
                            let mut guard = state.lock().unwrap();
                            guard.note_disconnected();
                        }
                        reply(m, json!({"ok": false, "error": e}))
                    }
                }
            })
            .inarg::<i64, _>("chat_id")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        let handle = handle.clone();
        interface = interface.add_m(
            f.method("SendFile", (), move |m| {
                let (chat_id, path, caption) = m.msg.read3::<i64, &str, &str>().map_err(MethodErr::from)?;
                let path = path.trim().to_string();
                let caption = caption.trim().to_string();
                if path.is_empty() {
                    return reply(m, json!({"ok": false, "error": "Empty path"}));
                }
                let (client, authed) = {
                    let s = state.lock().unwrap();
                    let authed = s.status()["authenticated"].as_bool().unwrap_or(false) && s.status()["connected"].as_bool().unwrap_or(false);
                    (s.client.clone(), authed)
                };
                if !authed {
                    return reply(m, json!({"ok": false, "error": "Not connected"}));
                }

                let send_res = handle.block_on(async move {
                    let p = std::path::Path::new(&path);
                    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string();
                    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    let is_photo = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif");

                    if is_photo {
                        let up = client.get_photo_upload(1, false).await.map_err(|e| e.to_string())?;
                        let upload_url = up.payload.get("url")
                            .or_else(|| up.payload.get("info").and_then(|i| i.get(0)).and_then(|i| i.get("url")))
                            .and_then(|v| v.as_str()).ok_or("No upload url")?.to_string();
                        let file = tokio::fs::File::open(&path).await.map_err(|e| e.to_string())?;
                        let up_res = client.upload_photo(upload_url, file, fname.clone(), None).await;
                        let token = up_res.get("photoToken").and_then(|v| v.as_str()).ok_or_else(|| format!("Upload failed: {}", up_res))?.to_string();
                        let attach = json!({"_type":"PHOTO","photoToken": token});
                        let mut args = std::collections::HashMap::new();
                        args.insert("attaches".to_string(), json!([attach]));
                        let resp = client.send_message(chat_id, caption.clone(), Some(args)).await.map_err(|e| e.to_string())?;
                        let real_id = resp.payload.get("message").and_then(|m| m.get("id"))
                            .or_else(|| resp.payload.get("messages").and_then(|a| a.get(0)).and_then(|m| m.get("id")))
                            .or_else(|| resp.payload.get("chat").and_then(|c| c.get("lastMessage")).and_then(|m| m.get("id")))
                            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
                            .unwrap_or(0);
                        let local_url = format!("file://{}", path);
                        let display = if caption.is_empty() { "[Фото]".to_string() } else { caption.clone() };
                        Ok::<_, String>((display, "[Фото]".to_string(), json!([{"type":"photo","url":local_url}]).to_string(), real_id))
                    } else {
                        let up = client.get_file_upload(1, false).await.map_err(|e| e.to_string())?;
                        let (upload_url, file_id) = if let Some(info) = up.payload.get("info").and_then(|i| i.get(0)) {
                            let url = info.get("url").and_then(|v| v.as_str()).ok_or("No upload url")?.to_string();
                            let fid = info.get("fileId").and_then(|v| v.as_u64()).unwrap_or(0);
                            (url, fid)
                        } else {
                            let url = up.payload.get("url").and_then(|v| v.as_str()).ok_or("No upload url")?.to_string();
                            let fid = up.payload.get("fileId").and_then(|v| v.as_u64()).unwrap_or(0);
                            (url, fid)
                        };
                        let file = tokio::fs::File::open(&path).await.map_err(|e| e.to_string())?;
                        let up_res = client.upload_file(upload_url, file_id, file, fname.clone()).await;
                        if up_res.get("error").is_some() { return Err(format!("Upload failed: {}", up_res)); }
                        let fid = up_res.get("fileId").and_then(|v| v.as_u64()).unwrap_or(file_id);
                        let attach = json!({"_type":"FILE","fileId": fid, "fileName": fname});
                        let mut args = std::collections::HashMap::new();
                        args.insert("attaches".to_string(), json!([attach]));
                        let resp = client.send_message(chat_id, caption.clone(), Some(args)).await.map_err(|e| e.to_string())?;
                        let real_id = resp.payload.get("message").and_then(|m| m.get("id"))
                            .or_else(|| resp.payload.get("messages").and_then(|a| a.get(0)).and_then(|m| m.get("id")))
                            .or_else(|| resp.payload.get("chat").and_then(|c| c.get("lastMessage")).and_then(|m| m.get("id")))
                            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
                            .unwrap_or(0);
                        let display = if caption.is_empty() { format!("[Файл] {}", fname) } else { caption.clone() };
                        Ok::<_, String>((display, format!("[Файл] {}", fname), json!([{"type":"file","url":"","name":fname,"size":0}]).to_string(), real_id))
                    }
                });

                match send_res {
                    Ok((msg_text, last, atts, real_id)) => {
                        let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                        let id = if real_id != 0 { real_id } else { -millis };
                        let guard = state.lock().unwrap();
                        let _ = guard.storage.add_message(chat_id, id, &msg_text, true, 0, &atts, 1);
                        let chats = guard.storage.get_chats().unwrap_or_default();
                        let title = chats.iter().find(|(cid,_,_,_,_,_)| *cid==chat_id).map(|(_,t,_,_,_,_)| t.clone()).unwrap_or_else(|| format!("Chat {chat_id}"));
                        let _ = guard.storage.set_chat_info(chat_id, &title, &last, "", 0);
                        reply(m, json!({"ok": true, "chat_id": chat_id, "message_id": id}))
                    }
                    Err(e) => {
                        let es = e.to_string();
                        if es.contains("Not connected") || es.contains("Connection closed") || es.contains("ping") {
                            let mut guard = state.lock().unwrap();
                            guard.note_disconnected();
                        }
                        reply(m, json!({"ok": false, "error": es}))
                    }
                }
            })
            .inarg::<i64, _>("chat_id")
            .inarg::<&str, _>("path")
            .inarg::<&str, _>("caption")
            .outarg::<&str, _>("reply"),
        );
    }

    {
        let state = state.clone();
        let handle = handle.clone();
        interface = interface.add_m(
            f.method("GetFolders", (), move |m| {
                let (client, authed) = {
                    let s = state.lock().unwrap();
                    let authed = s.status()["authenticated"].as_bool().unwrap_or(false) && s.status()["connected"].as_bool().unwrap_or(false);
                    (s.client.clone(), authed)
                };
                if !authed {
                    return reply(m, json!({"ok": true, "folders": []}));
                }
                let res = handle.block_on(async move { client.get_folders(None).await });
                match res {
                    Ok(resp) => {
                        let p = &resp.payload;
                        let folders = p.get("folders").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let order = p.get("foldersOrder").or_else(|| p.get("folders_order")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let mut out: Vec<Value> = Vec::new();
                        // respect order if possible
                        let mut by_id: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
                        for f in &folders {
                            if let Some(id) = f.get("id").and_then(|v| v.as_str()) {
                                by_id.insert(id.to_string(), f.clone());
                            }
                        }
                        let mut push_folder = |f: &Value| {
                            let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let title = f.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let include = f.get("include").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            let filters = f.get("filters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            out.push(json!({"id": id, "title": title, "include": include, "filters": filters}));
                        };
                        if !order.is_empty() {
                            for oid in &order {
                                if let Some(id) = oid.as_str() {
                                    if let Some(f) = by_id.remove(id) { push_folder(&f); }
                                }
                            }
                            for (_, f) in by_id { push_folder(&f); }
                        } else {
                            for f in &folders { push_folder(f); }
                        }
                        reply(m, json!({"ok": true, "folders": out}))
                    }
                    Err(e) => {
                        let es = e.to_string();
                        if es.contains("Not connected") || es.contains("Connection closed") || es.contains("ping") {
                            let mut guard = state.lock().unwrap();
                            guard.note_disconnected();
                        }
                        reply(m, json!({"ok": false, "error": es, "folders": []}))
                    }
                }
            })
            .outarg::<&str, _>("reply"),
        );
    }

    interface
}