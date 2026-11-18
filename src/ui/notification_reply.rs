//! Notification actions resolve an opaque token on the GTK thread. Neither
//! opening the composer nor sending from it changes the active conversation.
use super::*;
use crate::protocol::MessageOperationState;
use serde_json::json;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TOKEN_LIFETIME: Duration = Duration::from_secs(120);
const MAX_CONTEXTS: usize = 64;
const MAX_DIALOGS: usize = 8;

#[derive(Clone)]
struct Context {
    account: String,
    generation: u64,
    chat_mid: String,
    message_id: String,
    chat_name: String,
    preview: String,
    created: Instant,
    can_reply: bool,
}

impl Context {
    fn current(&self, state: &AppState) -> bool {
        state.self_mid.borrow().as_deref() == Some(self.account.as_str())
            && state.sidecar.session_generation() == self.generation
    }
}

#[derive(Default)]
pub(super) struct ReplyState {
    contexts: HashMap<String, Context>,
    dialogs: HashMap<String, Rc<ReplyUi>>,
    // GIO has no close event. Explicitly expire/withdraw its fallback bubbles.
    fallbacks: HashMap<String, String>,
}

pub(super) struct ReplyUi {
    window: gtk::Window,
    context: Context,
    text: gtk::TextView,
    send: gtk::Button,
    status: gtk::Label,
    unquoted: gtk::CheckButton,
    outcome: Cell<MessageOperationState>,
    source_unavailable: Cell<bool>,
    stale: Cell<bool>,
}

pub(super) fn register(
    state: &AppState,
    message: &MessageInfo,
    chat_mid: &str,
    chat_name: &str,
) -> Option<String> {
    let account = state.self_mid.borrow().clone()?;
    if !*state.session_ready.borrow() {
        return None;
    }
    let can_reply = crate::read_receipts::message_id(&message.id).is_some()
        && !message.decryption_failed
        && message.chat_event.is_none()
        && !["UNSENT", "18", "CALL", "CHAT_EVENT", "CHATEVENT"]
            .contains(&message.content_type.to_uppercase().as_str());
    let mut replies = state.notification_replies.borrow_mut();
    replies.contexts.retain(|_, context| {
        context.created.elapsed() < TOKEN_LIFETIME
            && context.current(state)
            && context.chat_mid != chat_mid
    });
    if replies.contexts.len() >= MAX_CONTEXTS
        && let Some(oldest) = replies
            .contexts
            .iter()
            .min_by_key(|(_, c)| c.created)
            .map(|(t, _)| t.clone())
    {
        replies.contexts.remove(&oldest);
    }
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let token = format!(
        "{}-{nonce}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    replies.contexts.insert(
        token.clone(),
        Context {
            account,
            generation: state.sidecar.session_generation(),
            chat_mid: chat_mid.into(),
            message_id: message.id.clone(),
            chat_name: chat_name.into(),
            preview: preview_body_ui(message).chars().take(512).collect(),
            created: Instant::now(),
            can_reply,
        },
    );
    drop(replies);
    let s = state.clone();
    let expires = token.clone();
    glib::timeout_add_local_once(TOKEN_LIFETIME, move || retire(&s, &expires));
    Some(token)
}

fn context(state: &AppState, token: &str) -> Option<Context> {
    state
        .notification_replies
        .borrow()
        .contexts
        .get(token)
        .filter(|c| c.current(state) && c.created.elapsed() < TOKEN_LIFETIME)
        .cloned()
}

pub(super) fn can_reply(state: &AppState, token: Option<&str>) -> bool {
    token
        .and_then(|token| context(state, token))
        .is_some_and(|context| context.can_reply)
}

pub(super) fn retire(state: &AppState, token: &str) {
    let mut replies = state.notification_replies.borrow_mut();
    replies.contexts.remove(token);
    let ids: Vec<_> = replies
        .fallbacks
        .iter()
        .filter(|(_, t)| t.as_str() == token)
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        replies.fallbacks.remove(&id);
        state.app.withdraw_notification(&id);
    }
}

pub(super) fn fallback_id(state: &AppState, token: Option<&str>) -> String {
    let id = context(state, token.unwrap_or_default())
        .map(|c| format!("line-gtk-{}-{}-{}", c.account, c.generation, c.chat_mid))
        .unwrap_or_else(|| "line-gtk-message".into());
    if let Some(token) = token {
        state
            .notification_replies
            .borrow_mut()
            .fallbacks
            .insert(id.clone(), token.into());
    }
    id
}

pub(super) fn wire(state: &AppState) {
    for (name, reply) in [("open-notification", false), ("reply-notification", true)] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let s = state.clone();
        action.connect_activate(move |_, value| {
            if let Some(token) = value.and_then(|v| v.str()) {
                if reply {
                    open(&s, token);
                } else {
                    open_chat(&s, token);
                }
            }
        });
        state.app.add_action(&action);
    }
}

fn expired(state: &AppState) {
    present_main_window(state);
    toast(state, &crate::i18n::t("notification_reply_expired"));
}

pub(super) fn open_chat(state: &AppState, token: &str) {
    if let Some(context) = context(state, token) {
        present_and_open_chat(state, &context.chat_mid);
        retire(state, token);
    } else {
        expired(state);
    }
}

pub(super) fn open(state: &AppState, token: &str) {
    let existing = state
        .notification_replies
        .borrow()
        .dialogs
        .get(token)
        .cloned();
    if let Some(ui) = existing {
        ui.window.present();
        return;
    }
    let Some(context) = context(state, token).filter(|context| context.can_reply) else {
        expired(state);
        return;
    };
    if state.notification_replies.borrow().dialogs.len() >= MAX_DIALOGS {
        expired(state);
        return;
    }
    let window = gtk::Window::builder()
        .application(&state.app)
        .transient_for(&state.window)
        .title(crate::i18n::t("notification_reply"))
        .default_width(460)
        .default_height(360)
        .build();
    window.set_titlebar(Some(&gtk::HeaderBar::new()));
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let destination = gtk::Label::builder()
        .label(crate::i18n::tf(
            "notification_reply_to",
            &[("chat", &context.chat_name)],
        ))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["heading"])
        .build();
    body.append(&destination);
    body.append(
        &gtk::Label::builder()
            .label(&context.preview)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build(),
    );
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .left_margin(10)
        .right_margin(10)
        .top_margin(10)
        .bottom_margin(10)
        .build();
    text.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "notification_reply_text",
    ))]);
    body.append(
        &gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(100)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["frame"])
            .child(&text)
            .build(),
    );
    let unquoted =
        gtk::CheckButton::with_label(&crate::i18n::t("notification_reply_without_quote"));
    unquoted.set_visible(false);
    if let Some(label) = unquoted.child().and_downcast::<gtk::Label>() {
        label.set_wrap(true);
    }
    body.append(&unquoted);
    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    body.append(&status);
    let actions = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::End)
        .build();
    let close = gtk::Button::with_label(&crate::i18n::t("notification_reply_close"));
    let send = gtk::Button::builder()
        .label(crate::i18n::t("send"))
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    send.set_tooltip_text(Some(&crate::i18n::t("notification_reply_shortcut")));
    actions.append(&close);
    actions.append(&send);
    body.append(&actions);
    window.set_child(Some(&body));
    let ui = Rc::new(ReplyUi {
        window: window.clone(),
        context,
        text,
        send,
        status,
        unquoted,
        outcome: Cell::new(MessageOperationState::Pending),
        source_unavailable: Cell::new(false),
        stale: Cell::new(false),
    });
    let weak = Rc::downgrade(&ui);
    let s = state.clone();
    ui.text.buffer().connect_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            refresh(&s, &ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    let s = state.clone();
    ui.unquoted.connect_toggled(move |_| {
        if let Some(ui) = weak.upgrade() {
            refresh(&s, &ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    let s = state.clone();
    ui.send.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            submit(&s, &ui);
        }
    });
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("notification-reply-keys"));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&ui);
    let s = state.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if let Some(ui) = weak.upgrade() {
            if key == gdk::Key::Escape {
                ui.window.close();
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            {
                submit(&s, &ui);
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(&ui);
    let s = state.clone();
    let token_owned = token.to_owned();
    window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            if ui.outcome.get() == MessageOperationState::Sending {
                return glib::Propagation::Stop;
            }
            let closing = Rc::downgrade(&ui);
            s.pending.borrow_mut().retain(|_, pending| !matches!(pending, Pending::NotificationReply { ui } if ui.ptr_eq(&closing)));
        }
        s.notification_replies
            .borrow_mut()
            .dialogs
            .remove(&token_owned);
        retire(&s, &token_owned);
        glib::Propagation::Proceed
    });
    state
        .notification_replies
        .borrow_mut()
        .dialogs
        .insert(token.into(), ui.clone());
    refresh(state, &ui);
    window.present();
    ui.text.grab_focus();
}

fn text(ui: &ReplyUi) -> String {
    let buffer = ui.text.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn refresh(state: &AppState, ui: &ReplyUi) {
    let editable = !ui.stale.get()
        && matches!(
            ui.outcome.get(),
            MessageOperationState::Pending | MessageOperationState::Failed
        );
    ui.text.set_editable(editable);
    ui.unquoted.set_sensitive(editable);
    ui.send.set_sensitive(
        editable
            && ui.context.current(state)
            && *state.session_ready.borrow()
            && !text(ui).trim().is_empty()
            && (!ui.source_unavailable.get() || ui.unquoted.is_active()),
    );
}

fn submit(state: &AppState, ui: &Rc<ReplyUi>) {
    if matches!(
        ui.outcome.get(),
        MessageOperationState::Sending
            | MessageOperationState::Unknown
            | MessageOperationState::Succeeded
    ) {
        return;
    }
    if !ui.context.current(state) || ui.stale.get() {
        ui.status
            .set_text(&crate::i18n::t("notification_reply_expired"));
        ui.stale.set(true);
        refresh(state, ui);
        return;
    }
    if !*state.session_ready.borrow() || !state.sidecar.status().running {
        ui.status
            .set_text(&crate::i18n::t("notification_reply_offline"));
        return;
    }
    refresh(state, ui);
    if !ui.send.is_sensitive() {
        return;
    }
    let quote = !ui.unquoted.is_active();
    let id = state.sidecar.request(
        "send_message",
        Some(json!({
            "chatMid": ui.context.chat_mid,
            "text": text(ui),
            "replyToMessageId": quote.then_some(&ui.context.message_id),
            "notificationReply": {
                "accountMid": ui.context.account,
                "sourceMessageId": ui.context.message_id,
                "quote": quote,
            },
        })),
    );
    match id {
        Ok(id) => {
            ui.outcome.set(MessageOperationState::Sending);
            ui.status
                .set_text(&crate::i18n::t("notification_reply_sending"));
            state.pending.borrow_mut().insert(
                id,
                Pending::NotificationReply {
                    ui: Rc::downgrade(ui),
                },
            );
        }
        Err(error) => {
            let unknown = error.to_string() == "send_outcome_unknown";
            ui.outcome.set(if unknown {
                MessageOperationState::Unknown
            } else {
                MessageOperationState::Failed
            });
            ui.status.set_text(&crate::i18n::t(if unknown {
                "send_outcome_unknown"
            } else {
                "notification_reply_offline"
            }));
        }
    }
    refresh(state, ui);
}

pub(super) fn response(
    state: &AppState,
    ui: &ReplyUi,
    ok: bool,
    result: &serde_json::Value,
    error: Option<&str>,
) {
    if !ui.context.current(state) || ui.stale.get() {
        return;
    }
    let outcome = MessageOperationState::from_response(ok, result, error);
    ui.outcome.set(outcome);
    let key = match outcome {
        MessageOperationState::Succeeded => "notification_reply_sent",
        MessageOperationState::Unknown | MessageOperationState::Sending => "send_outcome_unknown",
        _ => match error.unwrap_or_default() {
            "notification_reply_source_unavailable" | "invalid_reply_target" => {
                ui.source_unavailable.set(true);
                ui.unquoted.set_active(false);
                ui.unquoted.set_visible(true);
                "notification_reply_source_unavailable"
            }
            "contact_blocked_send" => "contact_blocked_send",
            "session_changed" | "not_logged_in" => "notification_reply_expired",
            _ => "notification_reply_failed",
        },
    };
    ui.status.set_text(&crate::i18n::t(key));
    refresh(state, ui);
    if outcome == MessageOperationState::Succeeded {
        ui.window.close();
    }
}

pub(super) fn unsent(state: &AppState, account: &str, chat_mid: &str, message_id: &str) {
    let dialogs: Vec<_> = state
        .notification_replies
        .borrow()
        .dialogs
        .values()
        .cloned()
        .collect();
    for ui in dialogs {
        if ui.context.account == account
            && ui.context.chat_mid == chat_mid
            && ui.context.message_id == message_id
        {
            ui.source_unavailable.set(true);
            ui.unquoted.set_visible(true);
            if matches!(
                ui.outcome.get(),
                MessageOperationState::Pending | MessageOperationState::Failed
            ) {
                ui.status
                    .set_text(&crate::i18n::t("notification_reply_source_unavailable"));
            }
            refresh(state, &ui);
        }
    }
}

/// Keep drafts readable after a process replacement, but never reuse its context.
pub(super) fn invalidate(state: &AppState) {
    let (dialogs, fallbacks) = {
        let mut replies = state.notification_replies.borrow_mut();
        replies.contexts.clear();
        (
            replies.dialogs.values().cloned().collect::<Vec<_>>(),
            std::mem::take(&mut replies.fallbacks),
        )
    };
    for id in fallbacks.keys() {
        state.app.withdraw_notification(id);
    }
    for ui in dialogs {
        ui.stale.set(true);
        ui.status.set_text(&crate::i18n::t(
            if matches!(
                ui.outcome.get(),
                MessageOperationState::Sending | MessageOperationState::Unknown
            ) {
                "send_outcome_unknown"
            } else {
                "notification_reply_expired"
            },
        ));
        if ui.outcome.get() == MessageOperationState::Sending {
            ui.outcome.set(MessageOperationState::Unknown);
        }
        refresh(state, &ui);
    }
}

pub(super) fn reset(state: &AppState) {
    invalidate(state);
    state
        .pending
        .borrow_mut()
        .retain(|_, pending| !matches!(pending, Pending::NotificationReply { .. }));
    let dialogs = std::mem::take(&mut state.notification_replies.borrow_mut().dialogs);
    for ui in dialogs.values() {
        ui.window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Own the notification bus name so this fixture never starts a real daemon
    // or displays synthetic messages on the user's desktop.
    struct FixtureNotifications {
        stop: std::sync::mpsc::Sender<()>,
        server: std::thread::JoinHandle<()>,
        calls: std::sync::Arc<std::sync::Mutex<Vec<(u32, String)>>>,
    }

    fn notification_daemon() -> FixtureNotifications {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
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
                    </interface></node>"#).unwrap();
                let sender_owner = std::cell::RefCell::new(None::<String>);
                connection.register_object("/org/freedesktop/Notifications", &node.lookup_interface("org.freedesktop.Notifications").unwrap())
                    .method_call(move |_, sender, _, _, method, params, invocation| {
                        if method == "GetCapabilities" {
                            invocation.return_value(Some(&(vec!["actions", "body"],).to_variant()));
                        } else if method == "Notify" {
                            if recorded.lock().unwrap().is_empty() {
                                *sender_owner.borrow_mut() = sender.map(str::to_owned);
                            } else {
                                assert_eq!(sender_owner.borrow().as_deref(), sender);
                            }
                            recorded.lock().unwrap().push((params.child_get::<u32>(1), params.child_get::<String>(4)));
                            invocation.return_value(Some(&(71_u32,).to_variant()));
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
        FixtureNotifications {
            stop: stop_tx,
            server,
            calls,
        }
    }

    fn source(id: &str) -> MessageInfo {
        serde_json::from_value(json!({"id": id, "from":"u-sender", "to":"c-reply", "text":"Bạn ăn trưa chưa?", "contentType":"NONE", "createdTime":1700000000000_i64})).unwrap()
    }

    #[test]
    #[ignore = "requires a desktop display and isolated D-Bus/XDG directories; fixture sidecar only"]
    fn fixture_notification_reply_context_drafts_and_outcomes() {
        use std::os::unix::fs::PermissionsExt;
        let FixtureNotifications {
            stop: stop_notifications,
            server: notification_server,
            calls: notification_calls,
        } = notification_daemon();
        libadwaita::init().unwrap();
        let root =
            std::env::temp_dir().join(format!("linegtk-notification-reply-{}", std::process::id()));
        std::fs::create_dir_all(root.join("protocol")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
            root.join("assets"),
        )
        .unwrap();
        AppConfig {
            language: "vi".into(),
            animations: false,
            tray_enabled: false,
            close_to_tray: false,
            ..AppConfig::default()
        }
        .save(&root.join("data"));
        let binary = root.join("protocol/line-gtk-protocol");
        std::fs::write(&binary, "#!/usr/bin/env python3\nimport json, sys, pathlib\nfor line in sys.stdin:\n    request = json.loads(line)\n    with (pathlib.Path(__file__).parent.parent / 'requests.jsonl').open('a') as log: log.write(line)\n    if request['method'] == 'fixture_exit': break\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = Application::builder()
            .application_id("dev.linegtk.NotificationReplyFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let state = build_ui(&app, root.clone(), root.join("data"), false).unwrap();
        *state.self_mid.borrow_mut() = Some("u-self".into());
        *state.session_ready.borrow_mut() = true;
        *state.current_chat.borrow_mut() = Some("c-other".into());
        state.stack.set_visible_child_name("shell");
        state.composer.set_text("Bản nháp ở cuộc trò chuyện khác");
        let message = source("9007199254740993123");
        let old = register(&state, &message, "c-reply", "Nhóm ăn trưa").unwrap();
        let newer = register(
            &state,
            &source("9007199254740993124"),
            "c-reply",
            "Nhóm ăn trưa",
        )
        .unwrap();
        assert!(context(&state, &old).is_none());
        state
            .app
            .activate_action("reply-notification", Some(&newer.to_variant()));
        let ui = state.notification_replies.borrow().dialogs[&newer].clone();
        assert_eq!(ui.context.message_id, "9007199254740993124");
        open(&state, &newer);
        assert_eq!(state.notification_replies.borrow().dialogs.len(), 1);
        ui.text.buffer().set_text("Mình chọn phở nhé.");
        assert!(ui.send.is_sensitive());
        assert_eq!(state.current_chat.borrow().as_deref(), Some("c-other"));
        assert_eq!(state.composer.text(), "Bản nháp ở cuộc trò chuyện khác");
        *state.session_ready.borrow_mut() = false;
        submit(&state, &ui);
        assert_eq!(ui.outcome.get(), MessageOperationState::Pending);
        assert_eq!(text(&ui), "Mình chọn phở nhé.");
        *state.session_ready.borrow_mut() = true;
        let controllers = ui.window.observe_controllers();
        let keys = (0..controllers.n_items())
            .filter_map(|i| controllers.item(i))
            .filter_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
            .find(|key| key.name().as_deref() == Some("notification-reply-keys"))
            .unwrap();
        assert!(keys.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::Return, &0_u32, &gdk::ModifierType::CONTROL_MASK]
        ));
        let request = *state
            .pending
            .borrow()
            .iter()
            .find(|(_, p)| matches!(p, Pending::NotificationReply { .. }))
            .unwrap()
            .0;
        let pending_count = state.pending.borrow().len();
        submit(&state, &ui);
        assert_eq!(state.pending.borrow().len(), pending_count);
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: request,
                ok: false,
                result: serde_json::Value::Null,
                error: Some("protocol request timed out".into()),
            },
        );
        assert_eq!(ui.outcome.get(), MessageOperationState::Unknown);
        unsent(&state, "u-self", "c-reply", "9007199254740993124");
        assert_eq!(ui.status.text(), crate::i18n::t("send_outcome_unknown"));
        assert!(!ui.send.is_sensitive());
        assert_eq!(text(&ui), "Mình chọn phở nhé.");
        assert!(state.pending.borrow().contains_key(&request));
        submit(&state, &ui);
        assert_eq!(state.pending.borrow().len(), pending_count);
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: request,
                ok: true,
                result: json!({"state":"succeeded", "sent":{"id":"9007199254740993125"}}),
                error: None,
            },
        );
        assert_eq!(ui.outcome.get(), MessageOperationState::Succeeded);
        assert!(!ui.send.is_sensitive());
        assert!(!state.pending.borrow().contains_key(&request));
        assert!(state.outbox.borrow().is_empty());
        assert_eq!(state.composer.text(), "Bản nháp ở cuộc trò chuyện khác");
        assert!(!ui.window.is_visible());
        assert_eq!(state.current_chat.borrow().as_deref(), Some("c-other"));
        assert!(
            !state
                .notification_replies
                .borrow()
                .dialogs
                .contains_key(&newer)
        );
        assert!(context(&state, &newer).is_none());

        // Rapid messages, including ones still awaiting sender details, become
        // one conversation notification with a bounded preview and newest reply.
        state.config.borrow_mut().notification_sound = false;
        state.config.borrow_mut().notifications = true;
        for id in ["101", "102", "103", "104"] {
            notifications::notify_incoming(&state, &source(id), "c-burst");
        }
        {
            let pending = state.notif_pending.borrow();
            assert_eq!(pending["101"].deliver_at, pending["104"].deliver_at);
            assert_eq!(pending["104"].recent_bodies.len(), 3);
            assert!(pending.values().all(|meta| !meta.notified));
            let token = pending["104"].reply_token.as_deref().unwrap();
            assert_eq!(context(&state, token).unwrap().message_id, "104");
        }
        let deadline = Instant::now() + Duration::from_millis(1400);
        while Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        {
            let pending = state.notif_pending.borrow();
            assert!(pending["104"].notified);
            assert_eq!(pending.values().filter(|meta| meta.notified).count(), 1);
        }
        assert_eq!(notification_calls.lock().unwrap().len(), 1);
        notifications::notify_incoming(&state, &source("105"), "c-burst");
        {
            let pending = state.notif_pending.borrow();
            assert!(pending["105"].coalesced);
            assert_eq!(pending["105"].recent_bodies.len(), 3);
        }
        let deadline = Instant::now() + Duration::from_millis(1400);
        while Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.notif_pending.borrow()["105"].notified);
        // A still-live popup must be replaced even after the grouping window,
        // and even if the bounded per-message preview cache has been discarded.
        state.notif_pending.borrow_mut().clear();
        notifications::notify_incoming(&state, &source("106"), "c-burst");
        let deadline = Instant::now() + Duration::from_millis(1400);
        while Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.notif_pending.borrow()["106"].recent_bodies.len(), 1);
        // Non-replyable notifications still retain a safe Open-chat context.
        let mut unavailable = message.clone();
        unavailable.decryption_failed = true;
        let token = register(&state, &unavailable, "c-reply", "Nhóm ăn trưa").unwrap();
        assert!(!can_reply(&state, Some(&token)));
        assert_eq!(context(&state, &token).unwrap().chat_mid, "c-reply");

        // A source removed from a different conversation is still reflected in
        // its open notification composer, without navigating the main window.
        let token = register(&state, &message, "c-reply", "Nhóm ăn trưa").unwrap();
        open(&state, &token);
        let ui = state.notification_replies.borrow().dialogs[&token].clone();
        ui.text.buffer().set_text("Mình chọn bún chả nhé.");
        unsent(&state, "u-other", "c-reply", &message.id);
        assert!(!ui.source_unavailable.get());
        events::handle_event(
            &state,
            ProtocolEvent::MessageUnsent {
                account_mid: "u-self".into(),
                chat_mid: "c-reply".into(),
                message_id: message.id.clone(),
            },
        );
        assert!(ui.source_unavailable.get());
        assert!(!ui.send.is_sensitive());
        assert!(ui.unquoted.is_visible());
        ui.unquoted.set_active(true);
        assert!(ui.send.is_sensitive());
        submit(&state, &ui);
        let request = *state
            .pending
            .borrow()
            .iter()
            .find(|(_, p)| matches!(p, Pending::NotificationReply { .. }))
            .unwrap()
            .0;
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: request,
                ok: false,
                result: json!({"state":"failed"}),
                error: Some("contact_blocked_send".into()),
            },
        );
        assert_eq!(ui.outcome.get(), MessageOperationState::Failed);
        assert_eq!(text(&ui), "Mình chọn bún chả nhé.");
        assert!(ui.send.is_sensitive());
        ui.unquoted.set_active(false);
        ui.status
            .set_text(&crate::i18n::t("notification_reply_source_unavailable"));
        for (scheme, file, width) in [
            (
                libadwaita::ColorScheme::ForceLight,
                "notification-reply-light.png",
                460,
            ),
            (
                libadwaita::ColorScheme::ForceDark,
                "notification-reply-dark-narrow.png",
                360,
            ),
        ] {
            libadwaita::StyleManager::default().set_color_scheme(scheme);
            ui.window.set_default_size(width, 400);
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                ui.window.width() as f64,
                ui.window.height() as f64,
            );
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gdk::Surface>).unwrap();
            renderer
                .render_texture(snapshot.to_node().unwrap(), None)
                .save_to_png(root.join(file))
                .unwrap();
            renderer.unrealize();
            let bounds = ui.send.compute_bounds(&ui.window).unwrap();
            assert!(bounds.x() + bounds.width() <= ui.window.width() as f32);
        }
        for language in ["en", "ja", "vi", "th"] {
            crate::i18n::set_lang(&root, language);
            for key in [
                "notification_reply",
                "notification_reply_source_unavailable",
                "notification_reply_expired",
                "notification_reply_offline",
            ] {
                assert_ne!(crate::i18n::t(key), key);
            }
        }
        crate::i18n::set_lang(&root, "vi");
        // Same-account process replacement invalidates tokens and open drafts.
        let stale = register(&state, &message, "u-peer", "Friend").unwrap();
        let mut replaced = ui.context.clone();
        replaced.generation = replaced.generation.wrapping_add(1);
        assert!(!replaced.current(&state));
        invalidate(&state);
        submit(&state, &ui);
        assert!(ui.stale.get());
        assert!(!ui.send.is_sensitive());
        assert_eq!(text(&ui), "Mình chọn bún chả nhé.");
        assert!(context(&state, &stale).is_none());
        ui.outcome.set(MessageOperationState::Unknown);
        invalidate(&state);
        invalidate(&state);
        submit(&state, &ui);
        assert_eq!(ui.status.text(), crate::i18n::t("send_outcome_unknown"));
        assert!(state.outbox.borrow().is_empty());
        *state.self_mid.borrow_mut() = Some("u-other".into());
        let count = state.pending.borrow().len();
        submit(&state, &ui);
        assert_eq!(state.pending.borrow().len(), count);
        reset(&state);
        assert!(state.notification_replies.borrow().dialogs.is_empty());
        let _ = state.sidecar.request("fixture_exit", None);
        // Let the recorder flush all requests before checking their wire context.
        std::thread::sleep(Duration::from_millis(100));
        let requests: Vec<serde_json::Value> = std::fs::read_to_string(root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let sends: Vec<_> = requests
            .iter()
            .filter(|r| r["method"] == "send_message")
            .collect();
        assert_eq!(sends.len(), 2);
        assert_eq!(sends[0]["params"]["chatMid"], "c-reply");
        assert_eq!(
            sends[0]["params"]["replyToMessageId"],
            "9007199254740993124"
        );
        assert_eq!(
            sends[0]["params"]["notificationReply"]["accountMid"],
            "u-self"
        );
        assert_eq!(
            sends[1]["params"]["replyToMessageId"],
            serde_json::Value::Null
        );
        assert_eq!(sends[1]["params"]["notificationReply"]["quote"], false);
        assert!(!requests.iter().any(|r| {
            r["method"]
                .as_str()
                .is_some_and(|method| method.contains("mark_read"))
        }));
        state.window.destroy();
        let calls = notification_calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, 0);
        assert_eq!(calls[1].0, 71);
        assert_eq!(calls[2].0, 71);
        assert_eq!(calls[0].1.lines().count(), 3);
        drop(calls);
        stop_notifications.send(()).unwrap();
        notification_server.join().unwrap();
        println!("Fixture screenshots: {}", root.display());
    }
}
