use super::*;
use crate::protocol::{SharedMediaDownload, SharedMediaItem};
use std::cell::Cell;

enum Item {
    Chat(Box<MessageInfo>),
    Shared {
        id: String,
        name: String,
        content_type: &'static str,
    },
}

impl Item {
    fn id(&self) -> &str {
        match self {
            Self::Chat(message) => &message.id,
            Self::Shared { id, .. } => id,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Chat(message) => suggest_media_name(message),
            Self::Shared { name, .. } => name.clone(),
        }
    }

    fn content_type(&self) -> &str {
        match self {
            Self::Chat(message) => &message.content_type,
            Self::Shared { content_type, .. } => content_type,
        }
    }

    fn kind(&self) -> ViewerKind {
        viewer_kind_for(self.content_type(), "", &self.name())
    }
}

pub(super) fn open_chat(state: &AppState, chat_mid: String, selected: MessageInfo) {
    let selected_id = selected.id.clone();
    let media = state.media_msgs.borrow();
    let mut items: Vec<_> = state
        .message_list
        .message_ids()
        .iter()
        .filter_map(|id| media.get(id))
        .filter(|msg| {
            !msg.id.is_empty()
                && !msg.id.starts_with("pending-")
                && matches!(
                    msg.content_type.to_ascii_uppercase().as_str(),
                    "IMAGE" | "VIDEO" | "AUDIO" | "FILE"
                )
        })
        .map(|msg| Item::Chat(Box::new(msg.clone())))
        .collect();
    drop(media);
    if !items.iter().any(|item| item.id() == selected_id) {
        items = vec![Item::Chat(Box::new(selected))];
    }
    open(state, &state.window, chat_mid, items, &selected_id);
}

pub(super) fn open_shared(
    state: &AppState,
    parent: &gtk::Window,
    chat_mid: &str,
    items: &[SharedMediaItem],
    selected_id: &str,
) {
    let items = items
        .iter()
        .filter_map(|item| {
            let content_type = match item.kind.as_str() {
                "image" => "IMAGE",
                "video" => "VIDEO",
                "audio" => "AUDIO",
                "file" => "FILE",
                _ => return None,
            };
            Some(Item::Shared {
                id: item.message_id.clone(),
                name: shared_media::item_name(item),
                content_type,
            })
        })
        .collect();
    open(state, parent, chat_mid.into(), items, selected_id);
}

pub(super) struct Gallery {
    viewer: MediaViewer,
    account: Option<String>,
    chat_mid: String,
    items: Vec<Item>,
    index: Cell<usize>,
    closed: Cell<bool>,
    pending: RefCell<HashSet<usize>>,
    downloaded: RefCell<HashMap<usize, SharedMediaDownload>>,
    previous: gtk::Button,
    next: gtk::Button,
}

fn open(
    state: &AppState,
    parent: &impl IsA<gtk::Window>,
    chat_mid: String,
    items: Vec<Item>,
    selected_id: &str,
) {
    let Some(index) = items.iter().position(|item| item.id() == selected_id) else {
        return;
    };
    let previous = arrow_button("go-previous-symbolic", "media_previous_file");
    let next = arrow_button("go-next-symbolic", "media_next_file");
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    navigation.append(&previous);
    navigation.append(&next);
    let (window, viewer) = MediaViewer::new(
        parent,
        items[index].kind(),
        (items.len() > 1).then_some(&navigation),
    );
    let gallery = Rc::new(Gallery {
        viewer,
        account: state.self_mid.borrow().clone(),
        chat_mid,
        items,
        index: Cell::new(index),
        closed: Cell::new(false),
        pending: RefCell::new(HashSet::new()),
        downloaded: RefCell::new(HashMap::new()),
        previous,
        next,
    });
    let weak = Rc::downgrade(&gallery);
    window.connect_unmap(move |_| {
        if let Some(gallery) = weak.upgrade() {
            gallery.closed.set(true);
        }
    });
    for (button, forward) in [(&gallery.previous, false), (&gallery.next, true)] {
        let weak = Rc::downgrade(&gallery);
        let s = state.clone();
        button.connect_clicked(move |_| {
            if let Some(gallery) = weak.upgrade() {
                gallery.step(&s, forward);
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = state.clone();
    let current = gallery.clone();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let forward = match key {
            gdk::Key::Left | gdk::Key::KP_Left => false,
            gdk::Key::Right | gdk::Key::KP_Right => true,
            _ => return glib::Propagation::Proceed,
        };
        if current.items.len() < 2
            || modifiers.intersects(gtk::accelerator_get_default_mod_mask())
            || weak
                .upgrade()
                .and_then(|win| GtkWindowExt::focus(&win))
                .is_some_and(|focus| {
                    focus.is::<gtk::Range>()
                        || focus.is::<gtk::Editable>()
                        || focus.ancestor(gtk::Popover::static_type()).is_some()
                })
        {
            return glib::Propagation::Proceed;
        }
        current.step(&s, forward);
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    gallery.show(state, index);
    window.present();
}

fn arrow_button(icon: &str, label_key: &str) -> gtk::Button {
    let label = crate::i18n::t(label_key);
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(&label)
        .css_classes(["flat", "circular"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
    button
}

impl Gallery {
    fn is_unsent(&self, state: &AppState, index: usize) -> bool {
        state.unsent_ids.borrow().contains(&(
            self.account.clone().unwrap_or_default(),
            self.items[index].id().into(),
        ))
    }

    fn neighbor(&self, state: &AppState, forward: bool) -> Option<usize> {
        let index = self.index.get();
        if forward {
            (index + 1..self.items.len()).find(|next| !self.is_unsent(state, *next))
        } else {
            (0..index).rev().find(|prev| !self.is_unsent(state, *prev))
        }
    }

    fn step(self: &Rc<Self>, state: &AppState, forward: bool) {
        if let Some(index) = self.neighbor(state, forward) {
            self.show(state, index);
        }
    }

    fn show(self: &Rc<Self>, state: &AppState, index: usize) {
        if self.account != *state.self_mid.borrow() {
            if let Some(window) = self.viewer.window.upgrade() {
                window.close();
            }
            return;
        }
        self.index.set(index);
        self.previous
            .set_sensitive(self.neighbor(state, false).is_some());
        self.next
            .set_sensitive(self.neighbor(state, true).is_some());
        let item = &self.items[index];
        if self.is_unsent(state, index) {
            self.status(state, "media_download_failed", false, false);
            return;
        }
        let cached = self.downloaded.borrow().get(&index).cloned();
        if let Some(file) = cached.filter(|file| std::path::Path::new(&file.path).is_file()) {
            self.show_file(state, &file);
            return;
        }
        let mut chat_message = None;
        if let Item::Chat(message) = item {
            let fresh = if state.current_chat.borrow().as_deref() == Some(self.chat_mid.as_str()) {
                state.media_msgs.borrow().get(item.id()).cloned()
            } else {
                None
            };
            let path = fresh
                .as_ref()
                .and_then(|message| local_media_path(message, true));
            if let Some(path) = path.or_else(|| local_media_path(message, true)) {
                self.viewer
                    .show_item(state, &path, item.content_type(), &item.name(), None);
                return;
            }
            chat_message = Some(item.id().into());
        }

        self.status(state, "media_downloading", true, false);
        if !self.pending.borrow_mut().insert(index) {
            return;
        }
        let request = match item {
            Item::Chat(message) => {
                state
                    .sidecar
                    .download_media(&self.chat_mid, &message.id, &message.content_type)
            }
            Item::Shared { id, .. } => state.sidecar.download_shared_media(&self.chat_mid, id),
        };
        match request {
            Ok(id) => {
                state.pending.borrow_mut().insert(
                    id,
                    Pending::PreviewDownload {
                        gallery: Rc::downgrade(self),
                        index,
                        chat_message,
                    },
                );
            }
            Err(_) => self.download_finished(state, index, false, &serde_json::Value::Null),
        }
    }

    pub fn download_finished(
        self: &Rc<Self>,
        state: &AppState,
        index: usize,
        ok: bool,
        result: &serde_json::Value,
    ) {
        self.pending.borrow_mut().remove(&index);
        if self.closed.get() || self.account != *state.self_mid.borrow() {
            return;
        }
        let item = &self.items[index];
        let unsent = self.is_unsent(state, index);
        let downloaded = if ok && !unsent {
            match item {
                Item::Chat(_) => result
                    .get("path")
                    .and_then(|value| value.as_str())
                    .map(|path| SharedMediaDownload {
                        path: path.into(),
                        file_name: item.name(),
                        content_type: item.content_type().into(),
                    }),
                Item::Shared { .. } => serde_json::from_value(result.clone()).ok(),
            }
            .filter(|file| std::path::Path::new(&file.path).is_file())
        } else {
            None
        };
        if let Some(file) = downloaded {
            if let Item::Chat(_) = item
                && state.current_chat.borrow().as_deref() == Some(self.chat_mid.as_str())
            {
                remember_media_download(state, item.id(), &file.path, &file.content_type);
            }
            if index == self.index.get() {
                self.show_file(state, &file);
            }
            self.downloaded.borrow_mut().insert(index, file);
        } else if index == self.index.get() {
            self.status(state, "media_download_failed", false, !unsent);
        }
    }

    fn show_file(&self, state: &AppState, file: &SharedMediaDownload) {
        self.viewer
            .show_item(state, &file.path, &file.content_type, &file.file_name, None);
    }

    fn status(self: &Rc<Self>, state: &AppState, label: &str, loading: bool, retry: bool) {
        let status = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .build();
        if loading {
            let spinner = gtk::Spinner::builder().spinning(true).build();
            status.append(&spinner);
        }
        status.append(
            &gtk::Label::builder()
                .label(crate::i18n::t(label))
                .accessible_role(gtk::AccessibleRole::Status)
                .build(),
        );
        if retry {
            let button = gtk::Button::with_label(&crate::i18n::t("retry"));
            let weak = Rc::downgrade(self);
            let s = state.clone();
            button.connect_clicked(move |_| {
                if let Some(gallery) = weak.upgrade() {
                    gallery.show(&s, gallery.index.get());
                }
            });
            status.append(&button);
        }
        let item = &self.items[self.index.get()];
        self.viewer.show_status(&item.name(), item.kind(), &status);
    }
}
