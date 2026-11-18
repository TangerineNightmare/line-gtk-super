//! デスクトップ通知。スタンプの画像表示と、同じ通知の更新を扱う。

use crate::tray::TrayAction;
use gtk::gio;
use gtk::prelude::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

#[derive(Debug, Clone)]
pub struct NotificationTicket {
    id: Arc<AtomicU32>,
    owner: String,
    _subscription: Rc<gio::SignalSubscription>,
    closed: Arc<AtomicBool>,
    token: Arc<Mutex<Option<String>>>,
}

impl NotificationTicket {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// 送信者アイコンと本文画像を持つ通知。クリックでトークを開く。
pub struct ChatNotification<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub avatar_path: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub reply_token: Option<&'a str>,
    pub allow_reply: bool,
    pub suppress_sound: bool,
    pub sticker: bool,
    pub replace: Option<&'a NotificationTicket>,
}

fn notification_body(
    body: &str,
    image: Option<&str>,
    sticker: bool,
    capabilities: &[String],
) -> String {
    if !capabilities.iter().any(|cap| cap == "body-markup") {
        return body.to_string();
    }
    let escaped = glib::markup_escape_text(body);
    if sticker
        && capabilities.iter().any(|cap| cap == "body-images")
        && let Some(uri) = image.and_then(|path| glib::filename_to_uri(path, None).ok())
    {
        return format!(
            "<img src=\"{}\" alt=\"{}\"/>",
            glib::markup_escape_text(&uri),
            escaped
        );
    }
    escaped.to_string()
}

/// 通知本文向けに縦100・横200ピクセル以内の静止画を作る。
pub fn sticker_preview(path: &Path, cache: &Path) -> Option<PathBuf> {
    let metadata = path.metadata().ok()?;
    if metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    metadata.len().hash(&mut hash);
    metadata.modified().ok()?.hash(&mut hash);
    let dest = cache.join(format!("notification-{:016x}.png", hash.finish()));
    if dest.is_file() {
        return Some(dest);
    }
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(200, 100);
    crate::config::ensure_private_dir(cache).ok()?;
    image.save(&dest).ok()?;
    Some(dest)
}

pub fn show_chat_notification(
    request: &ChatNotification<'_>,
    tx: async_channel::Sender<TrayAction>,
) -> Result<NotificationTicket, String> {
    let avatar = request
        .avatar_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());
    let image = request
        .image_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());

    // GIO caches this connection for the process. Notify, replacements and
    // action subscriptions must share the same D-Bus sender identity.
    let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    let capabilities =
        notification_call(&connection, "GetCapabilities", None)?.child_get::<Vec<String>>(0);
    let owner = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&("org.freedesktop.Notifications",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string())?
        .child_get::<String>(0);
    let replacement = request
        .replace
        .filter(|ticket| !ticket.is_closed() && ticket.owner == owner);
    let ticket = if let Some(ticket) = replacement {
        ticket.clone()
    } else {
        let id = Arc::new(AtomicU32::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let token = Arc::new(Mutex::new(None::<String>));
        let signal_id = id.clone();
        let signal_closed = closed.clone();
        let signal_token = token.clone();
        let subscription = connection.subscribe_to_signal(
            Some(&owner),
            Some("org.freedesktop.Notifications"),
            None,
            Some("/org/freedesktop/Notifications"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let action = match signal.signal_name {
                    "ActionInvoked" => signal.parameters.get::<(u32, String)>(),
                    "NotificationClosed" => signal
                        .parameters
                        .get::<(u32, u32)>()
                        .map(|(id, _)| (id, "__closed".into())),
                    _ => None,
                };
                let Some((id, action)) = action else {
                    return;
                };
                if id != signal_id.load(Ordering::Acquire)
                    || signal_closed.swap(true, Ordering::AcqRel)
                {
                    return;
                }
                let token = signal_token.lock().ok().and_then(|token| token.clone());
                if action == "default" {
                    let action = token
                        .clone()
                        .map(|token| TrayAction::OpenNotification { token })
                        .unwrap_or(TrayAction::Show);
                    let _ = tx.try_send(action);
                } else if let Some(token) = action.strip_prefix("reply:") {
                    let _ = tx.try_send(TrayAction::ReplyNotification {
                        token: token.into(),
                    });
                }
                if let Some(token) = token {
                    let _ = tx.try_send(TrayAction::NotificationClosed { token });
                }
            },
        );
        NotificationTicket {
            id,
            owner,
            closed,
            token,
            _subscription: Rc::new(subscription),
        }
    };
    let body = notification_body(request.body, image, request.sticker, &capabilities);
    let icon = if request.sticker && !capabilities.iter().any(|cap| cap == "body-images") {
        image.or(avatar)
    } else {
        avatar
    }
    .unwrap_or("line-gtk");
    let mut hints = HashMap::from([
        ("desktop-entry", "dev.linegtk.LineGtk".to_variant()),
        ("category", "im.received".to_variant()),
        ("transient", false.to_variant()),
        ("suppress-sound", request.suppress_sound.to_variant()),
    ]);
    if let Some(uri) = image.and_then(|path| glib::filename_to_uri(path, None).ok()) {
        hints.insert("image-path", uri.to_variant());
    }
    let mut actions = vec!["default".to_string(), "Open".to_string()];
    if request.allow_reply
        && let Some(token) = request.reply_token
    {
        actions.extend([
            format!("reply:{token}"),
            crate::i18n::t("notification_reply"),
        ]);
    }
    let result = notification_call(
        &connection,
        "Notify",
        Some(
            &(
                "LINE GTK",
                ticket.id.load(Ordering::Acquire),
                icon,
                request.title,
                body,
                actions,
                hints,
                12_000_i32,
            )
                .to_variant(),
        ),
    )?;
    // A restarted daemon may allocate a different ID. Always use its reply.
    ticket
        .id
        .store(result.child_get::<u32>(0), Ordering::Release);
    if let Ok(mut token) = ticket.token.lock() {
        *token = request.reply_token.map(str::to_owned);
    }
    Ok(ticket)
}

fn notification_call(
    connection: &gio::DBusConnection,
    method: &str,
    parameters: Option<&glib::Variant>,
) -> Result<glib::Variant, String> {
    connection
        .call_sync(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            method,
            parameters,
            None,
            gio::DBusCallFlags::NONE,
            3000,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticker_markup_uses_a_local_uri_and_escapes_alternative_text() {
        let caps = vec!["body-markup".into(), "body-images".into()];
        let body = notification_body("スタンプ<&\"", Some("/tmp/絵 空白.png"), true, &caps);
        assert!(body.starts_with("<img src=\"file:///tmp/"));
        assert!(body.contains("%20"));
        assert!(body.contains("alt=\"スタンプ&lt;&amp;&quot;\""));
        assert!(!body.contains(" width="));
    }

    #[test]
    fn unsupported_images_keep_text_and_user_text_cannot_add_markup() {
        let text = "<img src=\"https://example.invalid/x\"/> & 本文";
        assert_eq!(
            notification_body(text, Some("/tmp/sticker.png"), true, &[]),
            text
        );
        let markup = vec!["body-markup".into()];
        assert_eq!(
            notification_body("スタンプ", Some("/tmp/sticker.png"), true, &markup),
            "スタンプ"
        );
        let caps = vec!["body-markup".into(), "body-images".into()];
        let body = notification_body(text, Some("/tmp/photo.png"), false, &caps);
        assert!(!body.contains("<img"));
        assert!(body.starts_with("&lt;img"));
        assert_eq!(notification_body("スタンプ", None, true, &caps), "スタンプ");
    }

    #[test]
    fn preview_is_small_transparent_cached_and_rejects_invalid_images() {
        let temp = std::env::temp_dir().join(format!(
            "line-notify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temp).unwrap();
        let source = temp.join("スタンプ.png");
        image::RgbaImage::from_pixel(400, 300, image::Rgba([20, 80, 120, 0]))
            .save(&source)
            .unwrap();
        let cache = temp.join("cache");
        let path = sticker_preview(&source, &cache).unwrap();
        let preview = image::open(&path).unwrap().into_rgba8();
        assert!(preview.width() <= 200 && preview.height() <= 100);
        assert_eq!(preview.get_pixel(0, 0)[3], 0);
        assert_eq!(sticker_preview(&source, &cache), Some(path));
        std::fs::write(&source, "画像ではない").unwrap();
        assert!(sticker_preview(&source, &cache).is_none());
        std::fs::remove_dir_all(temp).unwrap();
    }
}

#[cfg(test)]
mod dbus_tests {
    use super::*;
    use gtk::gio;
    use std::time::Duration;

    #[test]
    #[ignore = "requires an isolated D-Bus session; installs a fixture notification daemon"]
    fn fixture_replaced_notification_routes_the_new_reply_token() {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None::<&gio::DBusAuthObserver>, None::<&gio::Cancellable>,
                ).unwrap();
                let node = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.Notifications">
                    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
                    <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
                    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
                    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
                    </interface></node>"#).unwrap();
                let calls = std::rc::Rc::new(std::cell::Cell::new(0));
                let calls_clone = calls.clone();
                let sender_owner = std::cell::RefCell::new(None::<String>);
                connection.register_object("/org/freedesktop/Notifications", &node.lookup_interface("org.freedesktop.Notifications").unwrap())
                    .method_call(move |connection, sender, _, _, method, params, invocation| {
                        if method == "GetCapabilities" {
                            invocation.return_value(Some(&(vec!["actions", "body"],).to_variant()));
                        } else if method == "Notify" {
                            let count = calls_clone.get() + 1;
                            calls_clone.set(count);
                            let actions = params.child_get::<Vec<String>>(5);
                            if count == 1 {
                                *sender_owner.borrow_mut() = sender.map(str::to_owned);
                                assert_eq!(params.child_get::<u32>(1), 0);
                                assert!(actions.iter().any(|a| a == "reply:old-context"));
                            } else {
                                assert_eq!(sender_owner.borrow().as_deref(), sender);
                                assert_eq!(params.child_get::<u32>(1), 71);
                                assert!(actions.iter().any(|a| a == "reply:new-context"));
                                let connection = connection.clone();
                                let destination = sender.unwrap().to_string();
                                glib::spawn_future_local(async move {
                                    glib::timeout_future(Duration::from_millis(150)).await;
                                    connection.emit_signal(Some(&destination), "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked", Some(&(72_u32, "reply:new-context").to_variant())).unwrap();
                                });
                            }
                            invocation.return_value(Some(&(if count == 1 { 71_u32 } else { 72_u32 },).to_variant()));
                        }
                    }).build().unwrap();
                let reply = connection.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", Some(&("org.freedesktop.Notifications", 4_u32).to_variant()), None, gio::DBusCallFlags::NONE, 3000, None::<&gio::Cancellable>).unwrap();
                assert_eq!(reply.child_get::<u32>(0), 1);
                ready_tx.send(()).unwrap();
                while stop_rx.try_recv().is_err() {
                    while context.pending() { context.iteration(false); }
                    std::thread::sleep(Duration::from_millis(2));
                }
                connection.close_sync(None::<&gio::Cancellable>).unwrap();
            }).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (tx, rx) = async_channel::unbounded();
        let request = ChatNotification {
            title: "Fixture",
            body: "Original",
            avatar_path: None,
            image_path: None,
            reply_token: Some("old-context"),
            allow_reply: true,
            suppress_sound: true,
            sticker: false,
            replace: None,
        };
        let first = show_chat_notification(&request, tx.clone()).unwrap();
        let replacement = ChatNotification {
            body: "New message",
            reply_token: Some("new-context"),
            replace: Some(&first),
            ..request
        };
        let second = show_chat_notification(&replacement, tx).unwrap();
        assert_eq!(
            first.id.load(Ordering::Acquire),
            second.id.load(Ordering::Acquire)
        );
        assert_eq!(second.id.load(Ordering::Acquire), 72);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let action = loop {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            if let Ok(action) = rx.try_recv() {
                break action;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "notification action timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            matches!(action, TrayAction::ReplyNotification { token } if token == "new-context")
        );
        assert!(first.is_closed());
        assert!(second.is_closed());
        assert!(
            matches!(rx.recv_blocking().unwrap(), TrayAction::NotificationClosed { token } if token == "new-context")
        );
        stop_tx.send(()).unwrap();
        server.join().unwrap();
    }
}
