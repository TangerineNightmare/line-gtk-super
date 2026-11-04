use super::*;

struct Ui {
    dialog: libadwaita::Dialog,
    list: gtk::Box,
    account: String,
    chat_mid: String,
    message_id: String,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

pub(super) fn close() {
    if let Some(ui) = UI.with(|ui| ui.borrow_mut().take()) {
        ui.dialog.force_close();
    }
}

const MAX_AVATARS: usize = 4;

pub(super) struct Slot {
    send_unknown: bool,
    status: gtk::Label,
    readers: gtk::Button,
    people: Vec<(String, String, Option<gdk::Texture>)>,
}

pub(super) fn send_unknown(state: &AppState, id: &str) {
    if let Some(slot) = state.receipt_slots.borrow_mut().get_mut(id) {
        slot.send_unknown = true;
        slot.status.set_text(&crate::i18n::t("send_state_unknown"));
        slot.status.remove_css_class("line-msg-status");
        slot.status.add_css_class("caption");
        slot.status.add_css_class("heading");
        slot.status
            .set_tooltip_text(Some(&crate::i18n::t("send_outcome_unknown")));
        slot.readers.set_visible(false);
    }
    if let Some(row) = state.pending_rows.borrow().get(id) {
        row.remove_css_class("line-msg-pending-row");
        if let Some(outer) = row.first_child() {
            outer.remove_css_class("line-msg-pending");
        }
        row.set_tooltip_text(Some(&crate::i18n::t("send_outcome_unknown")));
    }
}

pub(super) fn metadata(state: &AppState, message: &MessageInfo) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::End)
        .build();
    let status = gtk::Label::builder()
        .css_classes(["line-msg-status"])
        .build();
    let readers = gtk::Button::builder()
        .css_classes(["flat", "line-readers"])
        .visible(false)
        .build();
    readers.set_cursor_from_name(Some("pointer"));
    let s = state.clone();
    let id = message.id.clone();
    readers.connect_clicked(move |_| open(&s, &id));
    let mut slot = Slot {
        send_unknown: false,
        status,
        readers,
        people: Vec::new(),
    };
    update(state, &mut slot, &message.id, message.created_time);
    row.append(&slot.status);
    row.append(&slot.readers);
    state
        .receipt_slots
        .borrow_mut()
        .insert(message.id.clone(), slot);
    row
}

fn texture(state: &AppState, mid: &str) -> Option<gdk::Texture> {
    if let Some(texture) = state.reader_avatars.borrow().get(mid) {
        return Some(texture.clone());
    }
    let path = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == mid)
        .and_then(|chat| chat.avatar_path.clone())?;
    let pixbuf = Pixbuf::from_file_at_scale(path, 48, 48, true).ok()?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);
    state
        .reader_avatars
        .borrow_mut()
        .insert(mid.into(), texture.clone());
    Some(texture)
}

fn avatar(name: &str, texture: Option<&gdk::Texture>, size: i32) -> libadwaita::Avatar {
    let avatar = libadwaita::Avatar::new(size, Some(name), true);
    avatar.set_custom_image(texture);
    avatar
        .upcast_ref::<gtk::Widget>()
        .set_accessible_role(gtk::AccessibleRole::Presentation);
    avatar
}

pub(super) fn avatar_ready(state: &AppState, mid: &str, path: &str) {
    if !*state.session_ready.borrow() {
        return;
    }
    if let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 48, 48, true) {
        state
            .reader_avatars
            .borrow_mut()
            .insert(mid.into(), gdk::Texture::for_pixbuf(&pixbuf));
        refresh_read_labels(state);
    }
}

pub(super) fn update(state: &AppState, slot: &mut Slot, id: &str, created_time: i64) {
    if slot.send_unknown {
        slot.status.set_text(&crate::i18n::t("send_state_unknown"));
        slot.status
            .set_tooltip_text(Some(&crate::i18n::t("send_outcome_unknown")));
        return;
    }
    let (text, tooltip, read) = receipt_status(state, id, created_time);
    slot.status.set_text(&if read {
        format_msg_time(created_time)
    } else {
        text
    });
    slot.status.set_tooltip_text(Some(&tooltip));
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    let chat = state.current_chat.borrow().clone().unwrap_or_default();
    let mids = if read {
        state.peer_reads.borrow().readers(&chat, id, &account)
    } else {
        Vec::new()
    };
    let people: Vec<_> = mids
        .iter()
        .take(MAX_AVATARS)
        .map(|mid| {
            (
                mid.clone(),
                message_senders::name(state, mid),
                texture(state, mid),
            )
        })
        .collect();
    let content = if slot.people != people || slot.readers.child().is_none() {
        let content = gtk::Box::builder().spacing(3).build();
        for (_, name, texture) in &people {
            let portrait = avatar(name, texture.as_ref(), 20);
            portrait.set_tooltip_text(Some(name));
            content.append(&portrait);
        }
        slot.readers.set_child(Some(&content));
        slot.people = people;
        content
    } else {
        slot.readers
            .child()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap()
    };
    if let Some(last) = content
        .last_child()
        .filter(|last| last.has_css_class("line-reader-overflow"))
    {
        content.remove(&last);
    }
    if mids.len() > MAX_AVATARS {
        content.append(
            &gtk::Label::builder()
                .label(format!("+{}", mids.len() - MAX_AVATARS))
                .css_classes(["line-reader-overflow"])
                .build(),
        );
    }
    let names = slot
        .people
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let description = format!("{tooltip}: {names}. {}", crate::i18n::t("readers_show"));
    slot.readers.set_tooltip_text(Some(&description));
    slot.readers
        .update_property(&[gtk::accessible::Property::Label(&description)]);
    slot.readers.set_visible(!mids.is_empty());
}

fn open(state: &AppState, id: &str) {
    if !*state.session_ready.borrow() {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    if state.peer_reads.borrow().count(&chat_mid, id, &account) == 0 {
        return;
    }
    close();
    let dialog = libadwaita::Dialog::builder()
        .title(crate::i18n::t("readers_title"))
        .content_width(360)
        .content_height(420)
        .build();
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    dialog.connect_closed(|dialog| {
        UI.with(|cell| {
            let mut ui = cell.borrow_mut();
            if ui.as_ref().is_some_and(|ui| ui.dialog == *dialog) {
                *ui = None;
            }
        });
    });
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            dialog: dialog.clone(),
            list,
            account,
            chat_mid,
            message_id: id.into(),
        });
    });
    refresh(state);
    dialog.present(Some(&state.window));
}

pub(super) fn refresh(state: &AppState) {
    // Clone handles before touching GTK, which can emit signals synchronously.
    let current = UI.with(|ui| {
        ui.borrow().as_ref().map(|ui| {
            (
                ui.dialog.clone(),
                ui.list.clone(),
                ui.account.clone(),
                ui.chat_mid.clone(),
                ui.message_id.clone(),
            )
        })
    });
    let Some((dialog, list, account, chat_mid, id)) = current else {
        return;
    };
    if state.self_mid.borrow().as_deref() != Some(&account)
        || state.current_chat.borrow().as_deref() != Some(&chat_mid)
        || !*state.session_ready.borrow()
    {
        close();
        return;
    }
    let readers = state.peer_reads.borrow().readers(&chat_mid, &id, &account);
    dialog.set_title(&crate::i18n::tf(
        "status_read_count",
        &[("n", &readers.len().to_string())],
    ));
    let mut people: Vec<_> = readers
        .into_iter()
        .map(|mid| (message_senders::name(state, &mid), mid))
        .collect();
    people.sort_by_cached_key(|(name, mid)| (name.to_lowercase(), mid.clone()));
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (name, mid) in people {
        let row = gtk::Box::builder().spacing(12).build();
        row.append(&avatar(&name, texture(state, &mid).as_ref(), 32));
        row.append(
            &gtk::Label::builder()
                .label(&name)
                .tooltip_text(&name)
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .build(),
        );
        list.append(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a desktop display; uses a fixture sidecar and no LINE account"]
    fn fixture_unknown_sends_keep_text_and_accept_late_confirmation() {
        use std::os::unix::fs::PermissionsExt;
        libadwaita::init().unwrap();
        let root = std::env::temp_dir().join(format!("linegtk-send-ui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("protocol")).unwrap();
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
            root.join("assets"),
        )
        .unwrap();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        AppConfig {
            language: "vi".into(),
            animations: false,
            tray_enabled: false,
            close_to_tray: false,
            ..AppConfig::default()
        }
        .save(&data);
        let binary = root.join("protocol/line-gtk-protocol");
        std::fs::write(&binary, "#!/usr/bin/env python3\nimport json, sys\nfor line in sys.stdin:\n    if json.loads(line)['method'] == 'fixture_exit': break\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = Application::builder()
            .application_id("dev.linegtk.SendFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let state = build_ui(&app, root.clone(), data, false).unwrap();
        *state.self_mid.borrow_mut() = Some("u-self".into());
        *state.session_ready.borrow_mut() = true;
        *state.current_chat.borrow_mut() = Some("c-group".into());
        state.chat_title.set_text("Nhóm bạn bè");
        state.stack.set_visible_child_name("shell");
        set_msg_state(&state, "list", None);
        let message: MessageInfo = serde_json::from_value(serde_json::json!({
            "text":"Hẹn mọi người lúc 7 giờ nhé!", "from":"u-self", "to":"c-group",
            "createdTime":1700000000000_i64, "mine":true, "contentType":"NONE"
        }))
        .unwrap();
        let first = state
            .sidecar
            .send_message("c-group", &message.text)
            .unwrap();
        begin_optimistic_send(&state, first, "c-group", message.clone());
        let first_placeholder = format!("pending-{first}");
        state.composer.set_text("Bản nháp tiếp theo");
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: first,
                ok: false,
                result: serde_json::json!({"state":"unknown"}),
                error: Some("send_outcome_unknown".into()),
            },
        );
        refresh_read_labels(&state);
        assert_eq!(state.composer.text(), "Bản nháp tiếp theo");
        assert_eq!(
            state.receipt_slots.borrow()[&first_placeholder]
                .status
                .text(),
            crate::i18n::t("send_state_unknown")
        );
        assert!(state.pending_rows.borrow().contains_key(&first_placeholder));
        assert!(state.outbox.borrow().is_empty());

        let second = state
            .sidecar
            .send_message("c-group", "Mình sẽ đặt bàn.")
            .unwrap();
        let mut later = message.clone();
        later.text = "Mình sẽ đặt bàn.".into();
        begin_optimistic_send(&state, second, "c-group", later.clone());
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: second,
                ok: false,
                result: serde_json::Value::Null,
                error: Some("protocol request timed out".into()),
            },
        );
        assert!(state.pending.borrow().contains_key(&second));
        later.id = "9007199254740993123".into();
        // A receive event can precede the send response; the server ID dedupes it.
        append_message(&state, &later, false);
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: second,
                ok: true,
                result: serde_json::json!({"state":"succeeded", "message": {
                    "id":later.id, "text":later.text, "mine":true, "to":"c-group",
                    "createdTime":1700000000001_i64, "contentType":"NONE"
                }}),
                error: None,
            },
        );
        assert!(!state.pending.borrow().contains_key(&second));
        assert!(
            !state
                .pending_rows
                .borrow()
                .contains_key(&format!("pending-{second}"))
        );
        assert_eq!(state.message_list.message_count(), 2);
        for language in ["en", "ja", "vi", "th"] {
            crate::i18n::set_lang(&root, language);
            refresh_read_labels(&state);
            assert_eq!(
                state.receipt_slots.borrow()[&first_placeholder]
                    .status
                    .text(),
                crate::i18n::t("send_state_unknown")
            );
            assert_ne!(crate::i18n::t("send_state_unknown"), "send_state_unknown");
        }
        crate::i18n::set_lang(&root, "vi");
        refresh_read_labels(&state);
        state.window.set_default_size(760, 640);
        for (scheme, file) in [
            (
                libadwaita::ColorScheme::ForceLight,
                "send-unknown-light.png",
            ),
            (libadwaita::ColorScheme::ForceDark, "send-unknown-dark.png"),
        ] {
            libadwaita::StyleManager::default().set_color_scheme(scheme);
            settle();
            let paintable = gtk::WidgetPaintable::new(Some(&state.window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                state.window.width() as f64,
                state.window.height() as f64,
            );
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gdk::Surface>).unwrap();
            renderer
                .render_texture(snapshot.to_node().unwrap(), None)
                .save_to_png(root.join(file))
                .unwrap();
            renderer.unrealize();
        }
        let third = state
            .sidecar
            .send_message("c-group", "old account")
            .unwrap();
        begin_optimistic_send(&state, third, "c-group", message);
        *state.self_mid.borrow_mut() = Some("u-new".into());
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: third,
                ok: true,
                result: serde_json::json!({"message":{"id":"9999","text":"old account"}}),
                error: None,
            },
        );
        assert!(!state.seen_msg_ids.borrow().contains("9999"));
        assert_eq!(state.composer.text(), "Bản nháp tiếp theo");
        *state.self_mid.borrow_mut() = Some("u-self".into());
        let fourth = state
            .sidecar
            .send_message("c-group", "old process")
            .unwrap();
        begin_optimistic_send(&state, fourth, "c-group", later);
        if let Some(Pending::Send { generation, .. }) = state.pending.borrow_mut().get_mut(&fourth)
        {
            *generation = generation.wrapping_sub(1);
        }
        // A stale process response is ignored even when its account matches.
        events::handle_event(
            &state,
            ProtocolEvent::Response {
                id: fourth,
                ok: true,
                result: serde_json::json!({"message":{"id":"9998","text":"old process"}}),
                error: None,
            },
        );
        assert!(!state.seen_msg_ids.borrow().contains("9998"));
        state.window.destroy();
        let _ = state.sidecar.request("fixture_exit", None);
        println!("Send screenshots: {}", root.display());
    }

    fn settle() {
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(350);
        while std::time::Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn names() -> Vec<String> {
        UI.with(|ui| {
            let ui = ui.borrow();
            let ui = ui.as_ref().expect("reader dialog is open");
            let mut names = Vec::new();
            let mut child = ui.list.first_child();
            while let Some(row) = child {
                names.push(
                    row.last_child()
                        .unwrap()
                        .downcast::<gtk::Label>()
                        .unwrap()
                        .text()
                        .to_string(),
                );
                child = row.next_sibling();
            }
            names
        })
    }

    #[test]
    #[ignore = "requires a desktop display; uses a fixture sidecar and no LINE account"]
    fn fixture_reader_avatars_update_and_open_full_list_without_crossing_accounts() {
        use std::os::unix::fs::PermissionsExt;
        libadwaita::init().unwrap();
        let root = std::env::temp_dir().join(format!("linegtk-readers-ui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("protocol")).unwrap();
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
            root.join("assets"),
        )
        .unwrap();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        AppConfig {
            language: "vi".into(),
            animations: false,
            tray_enabled: false,
            close_to_tray: false,
            ..AppConfig::default()
        }
        .save(&data);
        let binary = root.join("protocol/line-gtk-protocol");
        std::fs::write(&binary, "#!/usr/bin/env python3\nimport json, sys\nfor line in sys.stdin:\n    if json.loads(line)['method'] == 'fixture_exit': break\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app = Application::builder()
            .application_id("dev.linegtk.ReadersFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let state = build_ui(&app, root.clone(), data, false).unwrap();
        *state.self_mid.borrow_mut() = Some("u-self".into());
        *state.session_ready.borrow_mut() = true;
        *state.current_chat.borrow_mut() = Some("c-group".into());
        state.chat_title.set_text("Nhóm bạn bè");
        state.chat_subtitle.set_text("3 thành viên");
        state.stack.set_visible_child_name("shell");
        set_msg_state(&state, "list", None);
        let message: MessageInfo = serde_json::from_value(serde_json::json!({"id":"100", "from":"u-self", "to":"c-group", "text":"Hẹn mọi người lúc 7 giờ nhé!", "createdTime":1700000000000_i64, "mine":true, "contentType":"TEXT"})).unwrap();
        append_message(&state, &message, false);
        let button = state
            .receipt_slots
            .borrow()
            .get("100")
            .unwrap()
            .readers
            .clone();
        assert!(!button.is_visible());
        state
            .peer_reads
            .borrow_mut()
            .observe("c-group", "u-lan", "100", "u-self");
        state
            .peer_reads
            .borrow_mut()
            .observe("c-group", "u-other", "101", "u-self");
        state
            .sender_names
            .borrow_mut()
            .insert("u-lan".into(), "Lan <&>".into());
        refresh_read_labels(&state);
        assert!(button.is_visible());
        assert_eq!(
            state
                .receipt_slots
                .borrow()
                .get("100")
                .unwrap()
                .people
                .len(),
            1
        );
        // Pointer and keyboard activation both emit this native button signal.
        button.emit_clicked();
        assert_eq!(names(), ["Lan <&>"]);
        message_senders::apply(&state, "u-lan".into(), "Lan Nguyễn".into());
        assert_eq!(names(), ["Lan Nguyễn"]);
        state
            .peer_reads
            .borrow_mut()
            .observe("c-group", "u-minh", "100", "u-self");
        state
            .sender_names
            .borrow_mut()
            .insert("u-minh".into(), "Minh Trần".into());
        refresh_read_labels(&state);
        assert_eq!(names(), ["Lan Nguyễn", "Minh Trần"]);
        close();
        let image_path = root.join("reader.png");
        let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 48, 48).unwrap();
        pixbuf.fill(0x48a88aff);
        pixbuf.savev(&image_path, "png", &[]).unwrap();
        avatar_ready(&state, "u-lan", image_path.to_str().unwrap());
        assert!(
            state
                .receipt_slots
                .borrow()
                .get("100")
                .unwrap()
                .people
                .iter()
                .find(|(mid, _, _)| mid == "u-lan")
                .unwrap()
                .2
                .is_some()
        );
        for (mid, name) in [
            ("u-anh", "Anh Lê"),
            ("u-bao", "Bảo Nguyễn"),
            ("u-ha", "Hà Phạm"),
            ("u-nhi", "Nhi Trần"),
        ] {
            state
                .sender_names
                .borrow_mut()
                .insert(mid.into(), name.into());
            state
                .peer_reads
                .borrow_mut()
                .observe("c-group", mid, "100", "u-self");
        }
        refresh_read_labels(&state);
        assert_eq!(
            state
                .receipt_slots
                .borrow()
                .get("100")
                .unwrap()
                .people
                .len(),
            MAX_AVATARS
        );
        assert_eq!(
            button
                .child()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .text(),
            "+2"
        );
        button.emit_clicked();
        assert_eq!(names().len(), 6);
        close();
        let mut second = message.clone();
        second.id = "102".into();
        second.text = "Mình sẽ đặt bàn trước.".into();
        append_message(&state, &second, false);
        state
            .peer_reads
            .borrow_mut()
            .observe("c-group", "u-minh", "102", "u-self");
        refresh_read_labels(&state);
        assert_eq!(
            state
                .receipt_slots
                .borrow()
                .get("102")
                .unwrap()
                .people
                .len(),
            1
        );
        let mut third = second.clone();
        third.id = "103".into();
        third.text = "Ai đến trễ thì nhắn mình nhé.".into();
        append_message(&state, &third, false);
        assert!(
            !state
                .receipt_slots
                .borrow()
                .get("103")
                .unwrap()
                .readers
                .is_visible()
        );
        settle();
        let paintable = gtk::WidgetPaintable::new(Some(&state.window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            state.window.width() as f64,
            state.window.height() as f64,
        );
        let renderer = gtk::gsk::CairoRenderer::new();
        renderer.realize(None::<&gdk::Surface>).unwrap();
        renderer
            .render_texture(snapshot.to_node().unwrap(), None)
            .save_to_png(root.join("readers.png"))
            .unwrap();
        renderer.unrealize();
        button.emit_clicked();
        *state.self_mid.borrow_mut() = Some("u-another".into());
        refresh(&state);
        assert!(UI.with(|ui| ui.borrow().is_none()));
        close();
        *state.current_chat.borrow_mut() = Some("u-lan".into());
        *state.self_mid.borrow_mut() = Some("u-self".into());
        state
            .peer_reads
            .borrow_mut()
            .observe("u-lan", "u-lan", "100", "u-self");
        refresh_read_labels(&state);
        assert_eq!(
            state
                .receipt_slots
                .borrow()
                .get("100")
                .unwrap()
                .people
                .len(),
            1
        );
        *state.current_chat.borrow_mut() = Some("c-group".into());
        clear_messages(&state);
        assert!(state.receipt_slots.borrow().is_empty());
        state.window.destroy();
        let _ = state.sidecar.request("fixture_exit", None);
        println!("Readers screenshot: {}", root.join("readers.png").display());
    }
}
