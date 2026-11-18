use super::*;

/// Keep the name visible until a photo has decoded successfully.
pub(super) fn wrap(picture: &gtk::Picture, name: &str, size: i32) -> gtk::Overlay {
    let fallback = libadwaita::Avatar::new(size, Some(name.trim()), true);
    fallback
        .upcast_ref::<gtk::Widget>()
        .set_accessible_role(gtk::AccessibleRole::Presentation);
    fallback.set_opacity(if picture.paintable().is_some() {
        0.0
    } else {
        1.0
    });
    picture.connect_paintable_notify(glib::clone!(
        #[weak]
        fallback,
        move |picture| {
            fallback.set_opacity(if picture.paintable().is_some() {
                0.0
            } else {
                1.0
            });
        }
    ));
    let overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .child(&fallback)
        .build();
    overlay.add_overlay(picture);
    overlay.set_measure_overlay(picture, false);
    overlay.set_clip_overlay(picture, true);
    picture
        .bind_property("visible", &overlay, "visible")
        .sync_create()
        .build();
    overlay
}

pub(super) fn set_name(picture: &gtk::Picture, name: &str) {
    if let Some(overlay) = picture.parent().and_downcast::<gtk::Overlay>()
        && let Some(fallback) = overlay.child().and_downcast::<libadwaita::Avatar>()
    {
        fallback.set_text(Some(name.trim()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle() {
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
        while std::time::Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "requires a desktop display and isolated XDG directories"]
    fn fixture_initials_survive_failed_photos_and_resize_without_layout_changes() {
        libadwaita::init().unwrap();
        shell::load_css();
        let root = std::env::temp_dir().join(format!("linegtk-avatars-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(16);
        row.set_margin_bottom(16);
        row.set_margin_start(16);
        row.set_margin_end(16);
        let mut portraits = Vec::new();
        for (size, name) in [
            (26, "Alice"),
            (32, "Alice Lee"),
            (36, "Nguyễn An"),
            (40, "山田 太郎"),
            (48, ""),
            (96, "Lan Anh"),
        ] {
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .build();
            let overlay = wrap(&picture, name, size);
            row.append(&overlay);
            portraits.push((picture, overlay, size));
        }
        let window = gtk::Window::builder().child(&row).build();
        window.present();
        settle();
        for (_, overlay, size) in &portraits {
            assert_eq!((overlay.width(), overlay.height()), (*size, *size));
            let fallback = overlay
                .child()
                .and_downcast::<libadwaita::Avatar>()
                .unwrap();
            assert_eq!(fallback.opacity(), 1.0);
        }
        let (picture, overlay, size) = &portraits[0];
        set_name(picture, "  Updated Name  ");
        assert_eq!(
            overlay
                .child()
                .and_downcast::<libadwaita::Avatar>()
                .unwrap()
                .text()
                .as_deref(),
            Some("Updated Name")
        );
        let chat: ChatInfo =
            serde_json::from_value(serde_json::json!({"mid": "u-test", "name": "Alice Lee"}))
                .unwrap();
        let (chat_row, chat_picture, _, _) = build_chat_row(&chat, false);
        let chat_box = chat_row.child().and_downcast::<gtk::Box>().unwrap();
        let chat_overlay = chat_box
            .first_child()
            .and_downcast::<gtk::Overlay>()
            .unwrap();
        for (compact, expected_size) in [(true, AVATAR_COMPACT_PX), (false, AVATAR_CHAT_PX)] {
            set_avatar_tier(&chat_overlay, compact);
            let fallback = chat_picture
                .parent()
                .and_downcast::<gtk::Overlay>()
                .unwrap()
                .child()
                .and_downcast::<libadwaita::Avatar>()
                .unwrap();
            assert_eq!(fallback.size(), expected_size);
        }
        let invalid = root.join("invalid.png");
        std::fs::write(&invalid, b"invalid image").unwrap();
        attach_texture_async(picture.clone(), invalid.to_str().unwrap().into(), 96);
        settle();
        assert!(picture.parent().is_some());
        assert_eq!(overlay.child().unwrap().opacity(), 1.0);
        let photo = root.join("photo.png");
        image::RgbaImage::from_pixel(240, 60, image::Rgba([50, 150, 90, 255]))
            .save(&photo)
            .unwrap();
        attach_texture_async(picture.clone(), photo.to_str().unwrap().into(), 96);
        settle();
        assert!(picture.paintable().is_some());
        assert_eq!(overlay.child().unwrap().opacity(), 0.0);
        assert_eq!((overlay.width(), overlay.height()), (*size, *size));
        picture.set_paintable(None::<&gdk::Paintable>);
        assert_eq!(overlay.child().unwrap().opacity(), 1.0);
        for (scheme, filename) in [
            (libadwaita::ColorScheme::ForceLight, "light.png"),
            (libadwaita::ColorScheme::ForceDark, "dark.png"),
        ] {
            libadwaita::StyleManager::default().set_color_scheme(scheme);
            settle();
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&row)).snapshot(
                &snapshot,
                row.width() as f64,
                row.height() as f64,
            );
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gdk::Surface>).unwrap();
            renderer
                .render_texture(snapshot.to_node().unwrap(), None)
                .save_to_png(root.join(filename))
                .unwrap();
            renderer.unrealize();
        }
        window.destroy();
    }
}
