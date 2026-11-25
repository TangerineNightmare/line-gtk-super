use super::*;

#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_image_viewer_context_menu_copies_full_image() {
    let (state, root) = fixture_state("image-copy");
    let path = root.join("picture.png");
    let image = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 800, 600).unwrap();
    image.fill(0x3388ccff);
    image.savev(&path, "png", &[]).unwrap();
    let viewer = gtk::Window::builder()
        .default_width(1000)
        .default_height(760)
        .build();
    let root_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .vexpand(true)
        .build();
    media::append_image_viewer(&state, &bar, &body, path.to_str().unwrap(), &viewer);
    root_box.append(&bar);
    root_box.append(&body);
    viewer.set_child(Some(&root_box));
    viewer.present();
    settle();
    // Copy stays bound to this viewer even after navigating away from its chat.
    *state.current_chat.borrow_mut() = None;
    let controllers = body.observe_controllers();
    let keys: Vec<_> = (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)?
                .downcast::<gtk::EventControllerKey>()
                .ok()
        })
        .collect();
    let click = (0..controllers.n_items())
        .find_map(|i| controllers.item(i)?.downcast::<gtk::GestureClick>().ok())
        .unwrap();
    assert_eq!(click.button(), 3);
    assert_eq!(click.propagation_phase(), gtk::PropagationPhase::Capture);
    for dark in [false, true] {
        libadwaita::StyleManager::default().set_color_scheme(if dark {
            libadwaita::ColorScheme::ForceDark
        } else {
            libadwaita::ColorScheme::ForceLight
        });
        click.emit_by_name::<()>("pressed", &[&1i32, &200f64, &200f64]);
        settle();
        let menu = find(body.upcast_ref(), &|w| w.is::<gtk::PopoverMenu>())
            .unwrap()
            .downcast::<gtk::PopoverMenu>()
            .unwrap();
        assert!(menu.is_mapped());
        let model = menu.menu_model().unwrap();
        assert_eq!(
            model.item_attribute_value(0, "label", None).unwrap().str(),
            Some(crate::i18n::t("copy_image").as_str())
        );
        let paintable = gtk::WidgetPaintable::new(Some(&menu));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, menu.width() as f64, menu.height() as f64);
        let renderer = gtk::gsk::CairoRenderer::new();
        renderer.realize(None::<&gdk::Surface>).unwrap();
        renderer
            .render_texture(snapshot.to_node().unwrap(), None)
            .save_to_png(root.join(if dark {
                "copy-menu-dark.png"
            } else {
                "copy-menu-light.png"
            }))
            .unwrap();
        renderer.unrealize();
        body.activate_action("image.copy", None).unwrap();
        let texture = glib::MainContext::default()
            .block_on(state.window.clipboard().read_texture_future())
            .unwrap()
            .unwrap();
        assert_eq!((texture.width(), texture.height()), (800, 600));
        let expected = gdk::Texture::from_filename(&path).unwrap();
        let mut actual_pixels = vec![0; 800 * 600 * 4];
        let mut expected_pixels = actual_pixels.clone();
        texture.download(&mut actual_pixels, 800 * 4);
        expected.download(&mut expected_pixels, 800 * 4);
        assert_eq!(actual_pixels, expected_pixels);
        menu.popdown();
        settle();
    }
    assert!(keys.iter().any(|key| key.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Menu, &0u32, &gdk::ModifierType::empty()]
    )));
    settle();
    let menu = find(body.upcast_ref(), &|w| w.is::<gtk::PopoverMenu>()).unwrap();
    assert!(menu.is_mapped());
    menu.downcast::<gtk::PopoverMenu>().unwrap().popdown();
    settle();
    state
        .window
        .clipboard()
        .set_text("replace previous clipboard");
    assert!(keys.iter().any(|key| key.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::c, &0u32, &gdk::ModifierType::CONTROL_MASK]
    )));
    let texture = glib::MainContext::default()
        .block_on(state.window.clipboard().read_texture_future())
        .unwrap()
        .unwrap();
    assert_eq!((texture.width(), texture.height()), (800, 600));
    viewer.close();
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    println!("Image copy screenshots: {}", root.display());
}

#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_file_preview_navigates_in_chat_order_and_ignores_late_downloads() {
    let (state, root) = fixture_state("file-preview");
    state.stack.set_visible_child_name("shell");
    set_msg_state(&state, "list", None);
    state.window.present();
    let image = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 600, 600).unwrap();
    image.fill(0x3388ccff);
    let image_path = root.join("first.png");
    image.savev(&image_path, "png", &[]).unwrap();
    let slow_path = root.join("slow.txt");
    std::fs::write(&slow_path, "Finished download").unwrap();
    let text_path = root.join("last.txt");
    std::fs::write(&text_path, "Cached document").unwrap();
    for (id, name, path) in [
        (30, "first.png", Some(&image_path)),
        (10, "slow.txt", None),
        (20, "last.txt", Some(&text_path)),
    ] {
        let mut file = message(id);
        file.created_time = 1700000000000;
        file.content_type = "FILE".into();
        file.file_name = Some(name.into());
        file.file_path = path.map(|path| path.to_str().unwrap().into());
        append_message(&state, &file, false);
        if id == 30 {
            append_message(&state, &message(99), false);
        }
    }
    let first_row = state.message_rows.borrow()["30"].clone();
    let file_card = find(first_row.upcast_ref(), &|w| {
        w.has_css_class("line-file-card")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    file_card.emit_clicked();
    settle();
    let viewer = visible_window(|window| window.has_css_class("line-media-viewer"));
    let title = preview_title(&viewer);
    let previous = preview_arrow(&viewer, "go-previous-symbolic");
    let next = preview_arrow(&viewer, "go-next-symbolic");
    let controllers = viewer.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)?
                .downcast::<gtk::EventControllerKey>()
                .ok()
        })
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .unwrap();
    let press =
        |key| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &gdk::ModifierType::empty()]);
    assert_eq!(title.text(), "first.png");
    assert!(!previous.is_sensitive());
    assert!(press(gdk::Key::Left));
    assert_eq!(title.text(), "first.png");

    let old_image = find(viewer.upcast_ref(), &|w| {
        w.has_css_class("line-media-viewer-image")
    })
    .unwrap()
    .downgrade();
    next.emit_clicked();
    assert_eq!(title.text(), "slow.txt");
    let request = state
        .pending
        .borrow()
        .iter()
        .find_map(|(id, request)| {
            matches!(request, Pending::PreviewDownload { chat_message: Some(message_id), .. }
            if message_id == "10")
            .then_some(*id)
        })
        .unwrap();
    assert!(press(gdk::Key::Right));
    assert_eq!(title.text(), "last.txt");
    assert!(!next.is_sensitive());
    assert!(previous.is_sensitive());
    handle_event(
        &state,
        ProtocolEvent::Response {
            id: request,
            ok: true,
            result: serde_json::json!({ "path": slow_path }),
            error: None,
        },
    );
    settle();
    assert_eq!(title.text(), "last.txt");
    assert!(old_image.upgrade().is_none());

    previous.emit_clicked();
    assert_eq!(title.text(), "slow.txt");
    let text = find(viewer.upcast_ref(), &|w| w.is::<gtk::TextView>())
        .unwrap()
        .downcast::<gtk::TextView>()
        .unwrap()
        .buffer();
    assert_eq!(
        text.text(&text.start_iter(), &text.end_iter(), false),
        "Finished download"
    );
    assert!(press(gdk::Key::Left));
    assert_eq!(title.text(), "first.png");
    assert!(!previous.is_sensitive());
    viewer.close();
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
}

#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_file_preview_follows_the_shared_media_page_and_retries_downloads() {
    let (state, root) = fixture_state("shared-file-preview");
    *state.session_ready.borrow_mut() = true;
    shared_media::open(&state);
    let shared_title = crate::i18n::t("shared_media");
    let shared = visible_window(|window| window.title().as_deref() == Some(shared_title.as_str()));
    let files = crate::i18n::t("shared_media_files");
    find(shared.upcast_ref(), &|w| {
        w.downcast_ref::<gtk::ToggleButton>()
            .is_some_and(|button| button.label().as_deref() == Some(files.as_str()))
    })
    .unwrap()
    .downcast::<gtk::ToggleButton>()
    .unwrap()
    .set_active(true);
    let page = state
        .pending
        .borrow()
        .iter()
        .find_map(|(id, request)| {
            matches!(
                request,
                Pending::SharedMedia {
                    request: shared_media::SharedMediaRequest::List { category: 2, .. },
                    ..
                }
            )
            .then_some(*id)
        })
        .unwrap();
    let items = [(30, "newer.txt"), (10, "older.txt")].map(|(id, name)| {
        serde_json::json!({
            "key": id.to_string(), "messageId": id.to_string(), "chatMid": "c-fixture",
            "createdTime": 1700000000000_i64 + id, "senderMid": "u-friend",
            "kind": "file", "fileName": name,
        })
    });
    handle_event(
        &state,
        ProtocolEvent::Response {
            id: page,
            ok: true,
            error: None,
            result: serde_json::json!({
                "chatMid": "c-fixture", "category": "files", "items": items, "scanState": "complete",
            }),
        },
    );
    find(shared.upcast_ref(), &|w| {
        w.is::<gtk::Button>()
            && w.tooltip_text()
                .is_some_and(|text| text.starts_with("newer.txt\n"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap()
    .emit_clicked();
    let viewer = visible_window(|window| window.has_css_class("line-media-viewer"));
    let title = preview_title(&viewer);
    let previous = preview_arrow(&viewer, "go-previous-symbolic");
    let next = preview_arrow(&viewer, "go-next-symbolic");
    let complete = |target, file: Option<&str>| {
        let id = state
            .pending
            .borrow()
            .iter()
            .find_map(|(id, request)| {
                matches!(request, Pending::PreviewDownload { index, chat_message: None, .. }
                if *index == target)
                .then_some(*id)
            })
            .unwrap();
        let result = file
            .map(|name| {
                let path = root.join(name);
                std::fs::write(&path, name).unwrap();
                serde_json::json!({ "path": path, "fileName": name, "contentType": "FILE" })
            })
            .unwrap_or_default();
        handle_event(
            &state,
            ProtocolEvent::Response {
                id,
                ok: file.is_some(),
                result,
                error: None,
            },
        );
        id
    };
    complete(0, Some("newer.txt"));
    assert_eq!(title.text(), "newer.txt");
    assert!(!previous.is_sensitive());
    next.emit_clicked();
    assert_eq!(title.text(), "older.txt");
    assert!(!next.is_sensitive());
    let failed = complete(1, None);
    let retry = crate::i18n::t("retry");
    find(viewer.upcast_ref(), &|w| {
        w.downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some(retry.as_str()))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap()
    .emit_clicked();
    assert_ne!(complete(1, Some("older.txt")), failed);
    previous.emit_clicked();
    assert_eq!(title.text(), "newer.txt");
    assert!(find(viewer.upcast_ref(), &|w| w.is::<gtk::TextView>()).is_some());
    viewer.close();
    shared.close();
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
}

fn visible_window(predicate: impl Fn(&gtk::Window) -> bool) -> gtk::Window {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.is_visible() && predicate(window))
        .unwrap()
}

fn preview_title(window: &gtk::Window) -> gtk::Label {
    find(window.upcast_ref(), &|w| w.has_css_class("heading"))
        .unwrap()
        .downcast()
        .unwrap()
}

fn preview_arrow(window: &gtk::Window, icon: &str) -> gtk::Button {
    find(window.upcast_ref(), &|w| {
        w.downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.icon_name().as_deref() == Some(icon))
    })
    .unwrap()
    .downcast()
    .unwrap()
}

pub(super) fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(250);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn find(widget: &gtk::Widget, matches: &impl Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, matches) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn status_row(state: &AppState) -> gtk::Widget {
    find(state.message_list.widget().upcast_ref(), &|widget| {
        widget.has_css_class("line-history-sentinel")
    })
    .expect("the history status must be visible at the top")
}

fn assert_status(state: &AppState, key: &str, loading: bool, failed: bool) {
    let row = status_row(state);
    let label = find(&row, &|widget| widget.is::<gtk::Label>())
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(label.text(), crate::i18n::t(key));
    let spinner = find(&row, &|widget| widget.is::<gtk::Spinner>())
        .unwrap()
        .downcast::<gtk::Spinner>()
        .unwrap();
    assert_eq!(spinner.is_spinning(), loading);
    assert_eq!(spinner.is_visible(), loading);
    let retry = find(&row, &|widget| widget.is::<gtk::Button>()).unwrap();
    assert_eq!(retry.is_visible(), failed);
    assert!(
        row.height() >= 52,
        "{key}: row height {}, mapped {}",
        row.height(),
        row.is_mapped()
    );
}

fn complete(state: &AppState, ok: bool, result: serde_json::Value) {
    let id = state.history.borrow().older_request.unwrap();
    let pending = state.pending.borrow_mut().remove(&id).unwrap();
    response(state, id, pending, ok, &result);
    settle();
}

fn message(id: u32) -> MessageInfo {
    serde_json::from_value(message_json(id)).unwrap()
}

pub(super) fn message_json(id: u32) -> serde_json::Value {
    serde_json::json!({
        "id": id.to_string(), "from": "u-friend", "to": "c-fixture",
        "text": format!("Tin nhắn {id}"), "createdTime": 1700000000000_i64 + i64::from(id) * 1000,
        "mine": false, "contentType": "TEXT",
    })
}

fn screenshot(state: &AppState, path: &std::path::Path) {
    let paintable = gtk::WidgetPaintable::new(Some(&state.window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        state.window.width() as f64,
        state.window.height() as f64,
    );
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gdk::Surface>).unwrap();
    let texture = renderer.render_texture(snapshot.to_node().unwrap(), None);
    texture.save_to_png(path).unwrap();
    renderer.unrealize();
}

pub(super) fn fixture_state(tag: &str) -> (AppState, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    libadwaita::init().unwrap();
    let root = std::env::temp_dir().join(format!("linegtk-history-{tag}-{}", std::process::id()));
    let protocol = root.join("protocol");
    let data = root.join("data");
    std::fs::create_dir_all(&protocol).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
        root.join("assets"),
    )
    .unwrap();
    AppConfig {
        language: "vi".into(),
        animations: true,
        tray_enabled: false,
        close_to_tray: false,
        ..AppConfig::default()
    }
    .save(&data);
    let binary = protocol.join("line-gtk-protocol");
    std::fs::write(
        &binary,
        "#!/usr/bin/env python3\nimport json, sys\nfor line in sys.stdin:\n    if json.loads(line)['method'] == 'fixture_exit': break\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = Application::builder()
        .application_id("dev.linegtk.HistoryFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let state = build_ui(&app, root.clone(), data, false).unwrap();
    *state.self_mid.borrow_mut() = Some("u-fixture".into());
    *state.current_chat.borrow_mut() = Some("c-fixture".into());
    (state, root)
}

#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_history_loading_retry_end_and_chat_switch() {
    let (state, root) = fixture_state("ui");
    state.stack.set_visible_child_name("shell");
    state.chat_title.set_text("Lịch sử tin nhắn");
    set_msg_state(&state, "list", None);
    for id in 41..=60 {
        append_message(&state, &message(id), false);
    }
    reset_for_latest(&state, true);
    arm(&state);
    state.window.present();
    state.message_scroll.vadjustment().set_value(0.0);
    settle();
    state.message_scroll.vadjustment().set_value(0.0);
    settle();
    screenshot(&state, &root.join("waiting-light.png"));
    assert_status(&state, "history_waiting", false, false);

    *state.session_ready.borrow_mut() = true;
    on_edge(&state, Edge::Top);
    settle();
    assert_status(&state, "history_older_loading", true, false);
    let request_id = state.history.borrow().older_request;
    on_edge(&state, Edge::Top);
    assert_eq!(state.history.borrow().older_request, request_id);
    for (name, scheme) in [
        ("loading-light.png", libadwaita::ColorScheme::ForceLight),
        ("loading-dark.png", libadwaita::ColorScheme::ForceDark),
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        settle();
        screenshot(&state, &root.join(name));
    }

    for failure in 1..=MAX_FAILURES {
        complete(&state, false, serde_json::json!({}));
        assert_status(&state, "history_failed", false, true);
        assert_eq!(state.history.borrow().older_failures, failure);
        state.history.borrow_mut().older_ready_at = None;
        on_edge(&state, Edge::Top);
        if failure < MAX_FAILURES {
            assert!(state.history.borrow().older_request.is_some());
        } else {
            assert!(state.history.borrow().older_request.is_none());
        }
    }
    screenshot(&state, &root.join("failed-dark.png"));
    let retry = find(&status_row(&state), &|widget| widget.is::<gtk::Button>())
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    retry.emit_clicked();
    settle();
    assert_status(&state, "history_older_loading", true, false);
    assert_eq!(state.history.borrow().older_failures, 0);

    // Exhausting a nonempty final page must keep both the messages and status.
    let anchor_row = state.message_rows.borrow().get("41").unwrap().clone();
    let before = anchor_row
        .compute_bounds(&state.message_scroll)
        .unwrap()
        .y();
    complete(
        &state,
        true,
        serde_json::json!({
            "messages": (31..41).map(message_json).collect::<Vec<_>>(), "hasMore": false,
        }),
    );
    let after = anchor_row
        .compute_bounds(&state.message_scroll)
        .unwrap()
        .y();
    assert!(
        (before - after).abs() < 2.0,
        "prepending history must preserve the reading position"
    );
    state.message_scroll.vadjustment().set_value(0.0);
    settle();
    assert_status(&state, "history_start", false, false);
    assert_eq!(state.message_list.message_count(), 30);
    assert!(state.seen_msg_ids.borrow().contains("31"));
    on_edge(&state, Edge::Top);
    assert!(state.history.borrow().older_request.is_none());
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceLight);
    settle();
    screenshot(&state, &root.join("exhausted-light.png"));

    // An old response must not change the loading row in the next chat.
    state.history.borrow_mut().has_more_older = true;
    state.history.borrow_mut().older_ready_at = None;
    state.history.borrow_mut().older_scroll_required = false;
    on_edge(&state, Edge::Top);
    let stale_id = state.history.borrow().older_request.unwrap();
    let stale_pending = state.pending.borrow_mut().remove(&stale_id).unwrap();
    clear_messages(&state);
    *state.history_request_generation.borrow_mut() += 1;
    *state.current_chat.borrow_mut() = Some("c-next".into());
    append_message(&state, &message(90), false);
    reset_for_latest(&state, true);
    arm(&state);
    on_edge(&state, Edge::Top);
    response(
        &state,
        stale_id,
        stale_pending,
        true,
        &serde_json::json!({"messages": [], "hasMore": false}),
    );
    settle();
    assert_status(&state, "history_older_loading", true, false);
    assert!(state.history.borrow().has_more_older);
    complete(
        &state,
        true,
        serde_json::json!({"messages": [], "hasMore": false}),
    );
    assert_status(&state, "history_start", false, false);

    // A partial page can leave a long list near the top. Restoring its anchor
    // must not request another page until the reader scrolls upward again.
    clear_messages(&state);
    *state.history_request_generation.borrow_mut() += 1;
    *state.current_chat.borrow_mut() = Some("c-prefetch".into());
    reset(&state);
    for id in 101..=140 {
        append_message(&state, &message(id), false);
    }
    settle();
    reset_for_latest(&state, true);
    arm(&state);
    state.message_scroll.vadjustment().set_value(0.0);
    settle();
    on_edge(&state, Edge::Top);
    let id = state.history.borrow().older_request.unwrap();
    let pending = state.pending.borrow_mut().remove(&id).unwrap();
    response(
        &state,
        id,
        pending,
        true,
        &serde_json::json!({"messages": [message_json(100)], "hasMore": true}),
    );
    assert!(*state.scroll_pinning.borrow());
    let adj = state.message_scroll.vadjustment();
    assert!(adj.upper() > adj.page_size() * 1.5);
    let deadline = Instant::now() + COOLDOWN + Duration::from_millis(500);
    while Instant::now() < deadline {
        settle();
    }
    assert!(near_edge(&state, Edge::Top));
    assert!(
        state.history.borrow().older_request.is_none(),
        "restoring the reading position must not keep loading older pages"
    );
    adj.set_value(adj.value() + 50.0);
    settle();
    assert!(
        state.history.borrow().older_request.is_none(),
        "scrolling away from the top must not request older history"
    );
    adj.set_value(0.0);
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.history.borrow().older_request.is_none() && Instant::now() < deadline {
        settle();
    }
    assert!(
        state.history.borrow().older_request.is_some(),
        "scrolling upward again must request the next page"
    );
    assert_status(&state, "history_older_loading", true, false);

    // Moving away before the response arrives must stop further prefetching.
    adj.set_value(adj.upper() - adj.page_size());
    settle();
    complete(
        &state,
        true,
        serde_json::json!({"messages": [message_json(99)], "hasMore": true}),
    );
    let deadline = Instant::now() + COOLDOWN + Duration::from_millis(500);
    while Instant::now() < deadline {
        settle();
    }
    assert!(!near_edge(&state, Edge::Top));
    assert!(state.history.borrow().older_request.is_none());

    // Start the next request while there are still two screens left to scroll.
    adj.set_value(adj.page_size() * 2.0);
    settle();
    assert!(state.history.borrow().older_request.is_some());
    complete(
        &state,
        true,
        serde_json::json!({"messages": [], "hasMore": false}),
    );

    // Prepending a full page while trimming the far end must preserve the
    // visible message, even though the list's total height barely changes.
    clear_messages(&state);
    *state.history_request_generation.borrow_mut() += 1;
    *state.current_chat.borrow_mut() = Some("c-bounded".into());
    reset(&state);
    for id in 201..=400 {
        append_message(&state, &message(id), false);
    }
    settle();
    reset_for_latest(&state, true);
    arm(&state);
    adj.set_value(adj.page_size() * 2.0);
    settle();
    on_edge(&state, Edge::Top);
    let (visible_row, before) = state
        .message_rows
        .borrow()
        .values()
        .filter_map(|row| {
            let rect = row.compute_bounds(&state.message_scroll)?;
            (row.is_mapped() && rect.y() >= 0.0 && rect.y() < adj.page_size() as f32)
                .then(|| (row.clone(), rect.y()))
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .expect("a message must be visible before prepending");
    complete(
        &state,
        true,
        serde_json::json!({
            "messages": (151..201).map(message_json).collect::<Vec<_>>(), "hasMore": true,
        }),
    );
    let deadline = Instant::now() + COOLDOWN + Duration::from_millis(500);
    while Instant::now() < deadline {
        settle();
    }
    let after = visible_row
        .compute_bounds(&state.message_scroll)
        .expect("the reading anchor must remain attached")
        .y();
    assert!(
        (before - after).abs() < 2.0,
        "trimming the far end must preserve the reading position: {before} -> {after}"
    );
    assert_eq!(state.message_list.message_count(), WINDOW_MAX);
    assert!(state.history.borrow().older_request.is_none());

    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    println!("History screenshots: {}", root.display());
}

/// A chat with only a few messages cannot be scrolled, so the first load is the
/// only chance to ask for older history. The top sentinel binds while the view
/// is still pinning to the newest message; that request must not be dropped.
#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_short_chat_loads_older_history_after_first_render() {
    let (state, root) = fixture_state("short");
    state.stack.set_visible_child_name("shell");
    set_msg_state(&state, "list", None);
    *state.session_ready.borrow_mut() = true;
    for id in 58..=60 {
        append_message(&state, &message(id), false);
    }
    reset_for_latest(&state, true);
    // The first render pins the list to the newest message for a short while.
    *state.scroll_pinning.borrow_mut() = true;
    let released = state.clone();
    glib::timeout_add_local_once(Duration::from_millis(300), move || {
        *released.scroll_pinning.borrow_mut() = false;
    });
    arm(&state);
    state.window.present();

    let adj = state.message_scroll.vadjustment();
    let deadline = Instant::now() + Duration::from_secs(3);
    while state.history.borrow().older_request.is_none() && Instant::now() < deadline {
        settle();
    }
    assert!(
        adj.upper() <= adj.page_size(),
        "the fixture must be too short to scroll"
    );
    assert!(
        state.history.borrow().older_request.is_some(),
        "a short chat must request older history without any scrolling"
    );

    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "requires a desktop display and isolated XDG directories; uses a fixture sidecar"]
fn fixture_navigation_does_not_repin_and_offline_jump_terminates() {
    let (state, root) = fixture_state("navigation");
    state.stack.set_visible_child_name("shell");
    set_msg_state(&state, "list", None);
    for id in 41..=120 {
        append_message(&state, &message(id), false);
    }
    reset_for_latest(&state, true);
    arm(&state);
    state.window.present();
    settle();
    let adj = state.message_scroll.vadjustment();
    adj.set_value(0.0);
    jump_to_message(&state, "60");
    let token = state
        .history
        .borrow()
        .navigation
        .as_ref()
        .unwrap()
        .token
        .clone();
    jump_to_message(&state, "60");
    assert!(Rc::ptr_eq(
        &token,
        &state.history.borrow().navigation.as_ref().unwrap().token
    ));
    adj.set_value((adj.upper() - adj.page_size()).max(0.0));
    let repinned = *state.stick_bottom.borrow();
    state.history.borrow_mut().navigation = None;
    jump_to_message(&state, "1");
    let offline_waiting = state.history.borrow().navigation.is_some();
    state.history.borrow_mut().navigation = None;
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    let _ = std::fs::remove_dir_all(root);
    assert!(!repinned, "navigation must not enable bottom pinning");
    assert!(
        !offline_waiting,
        "offline missing target must terminate navigation"
    );
}
