//! Message translation through a LibreTranslate-compatible HTTP API.
//!
//! `POST {translate_url}/translate` with `{q, source, target, format, api_key?}`
//! returning `{translatedText, detectedLanguage?}`. Any server speaking that API
//! works: a local LibreTranslate container (the default), a hosted LibreTranslate
//! instance, or a compatible proxy in front of another engine.
//!
//! Direction: text containing Thai script is translated from Thai; anything else
//! is sent with `source: "auto"` towards the reader's language. A message that is
//! already in the reader's language is translated into the partner language when
//! the user asks for it, and left alone by auto-translate.

use super::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;

const WIDGET_NAME: &str = "line-translation";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Translation {
    pub text: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Translated(Translation),
    /// Auto mode only: the message is already in the reader's language, or the
    /// server returned it unchanged.
    NotNeeded,
}

enum Entry {
    Shown(Translation),
    Hidden(Translation),
    NotNeeded,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    static BUBBLES: RefCell<HashMap<String, glib::WeakRef<gtk::Box>>> =
        RefCell::new(HashMap::new());
    static PENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static AUTO_ERROR_SHOWN: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn has_thai(text: &str) -> bool {
    text.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c))
}

/// Source and target for the first request.
pub(super) fn first_direction(text: &str, mine: &str, partner: &str) -> (String, String) {
    if has_thai(text) {
        let target = if mine == "th" { partner } else { mine };
        ("th".into(), target.into())
    } else {
        ("auto".into(), mine.into())
    }
}

#[derive(Clone)]
struct Job {
    url: String,
    api_key: String,
    text: String,
    mine: String,
    partner: String,
    auto: bool,
}

struct Reply {
    text: String,
    detected: Option<String>,
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("translation HTTP client")
    })
}

/// Parse a LibreTranslate `/translate` response body.
pub(super) fn parse_reply(body: &serde_json::Value) -> Result<(String, Option<String>), String> {
    if let Some(error) = body.get("error").and_then(|e| e.as_str()) {
        return Err(error.to_string());
    }
    let text = body
        .get("translatedText")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "response has no translatedText".to_string())?;
    // An object in LibreTranslate; tolerate an array from compatible servers.
    let detected = body.get("detectedLanguage").and_then(|d| {
        let d = d.as_array().and_then(|a| a.first()).unwrap_or(d);
        d.get("language")
            .and_then(|l| l.as_str())
            .map(str::to_string)
    });
    Ok((text.to_string(), detected))
}

fn post(job: &Job, source: &str, target: &str) -> Result<Reply, String> {
    let mut payload = serde_json::json!({
        "q": job.text,
        "source": source,
        "target": target,
        "format": "text",
    });
    if !job.api_key.is_empty() {
        payload["api_key"] = job.api_key.clone().into();
    }
    let response = client()
        .post(format!("{}/translate", job.url))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string())
        .send()
        .map_err(|e| {
            if e.is_connect() {
                format!("cannot reach {}", job.url)
            } else if e.is_timeout() {
                "timed out".to_string()
            } else {
                e.to_string()
            }
        })?;
    let status = response.status();
    let body: serde_json::Value = response
        .bytes()
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| format!("HTTP {} with a non-JSON body", status.as_u16()))?;
    if !status.is_success() && body.get("error").is_none() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let (text, detected) = parse_reply(&body)?;
    Ok(Reply { text, detected })
}

fn same_text(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn run(job: &Job) -> Result<Outcome, String> {
    let (source, target) = first_direction(&job.text, &job.mine, &job.partner);
    if source == target {
        return Ok(Outcome::NotNeeded);
    }
    let first = post(job, &source, &target)?;
    let from = first
        .detected
        .clone()
        .unwrap_or_else(|| source.clone())
        .to_ascii_lowercase();
    let from = from.split(['-', '_']).next().unwrap_or(&from).to_string();
    if from == job.mine || (from != "auto" && same_text(&first.text, &job.text)) {
        if job.auto || job.mine == job.partner {
            return Ok(Outcome::NotNeeded);
        }
        let second = post(job, &job.mine, &job.partner)?;
        if same_text(&second.text, &job.text) {
            return Ok(Outcome::NotNeeded);
        }
        return Ok(Outcome::Translated(Translation {
            text: second.text,
            from: job.mine.clone(),
            to: job.partner.clone(),
        }));
    }
    if same_text(&first.text, &job.text) {
        return Ok(Outcome::NotNeeded);
    }
    Ok(Outcome::Translated(Translation {
        text: first.text,
        from,
        to: target,
    }))
}

type Work = (Job, async_channel::Sender<Result<Outcome, String>>);

fn queue(job: Job, reply: async_channel::Sender<Result<Outcome, String>>) {
    static QUEUE: OnceLock<async_channel::Sender<Work>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = async_channel::bounded::<Work>(512);
        // Two workers: enough for auto-translate of a busy chat without
        // flooding a small local server.
        for _ in 0..2 {
            let rx = rx.clone();
            std::thread::spawn(move || {
                while let Ok((job, reply)) = rx.recv_blocking() {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send_blocking(run(&job));
                }
            });
        }
        tx
    });
    if let Err(error) = queue.try_send((job, reply)) {
        let (_, reply) = error.into_inner();
        let _ = reply.send_blocking(Err("translation queue is full".into()));
    }
}

fn job_for(state: &AppState, text: &str, auto: bool) -> Job {
    let config = state.config.borrow();
    Job {
        url: config.translate_url.clone(),
        api_key: config.translate_api_key.trim().to_string(),
        text: text.to_string(),
        mine: config.translate_language.clone(),
        partner: config.translate_partner_language.clone(),
        auto,
    }
}

pub(super) fn eligible(message: &MessageInfo) -> bool {
    message_copy::text_eligible(message) && message.text.chars().any(char::is_alphabetic)
}

fn remove_widget(bubble: &gtk::Box) {
    let mut child = bubble.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.widget_name() == WIDGET_NAME {
            bubble.remove(&widget);
        }
    }
}

fn label_box(caption: &str, text: Option<&str>) -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .css_classes(["line-translation"])
        .build();
    container.set_widget_name(WIDGET_NAME);
    container.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    container.append(
        &gtk::Label::builder()
            .label(caption)
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    if let Some(text) = text {
        container.append(
            &gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(42)
                .selectable(true)
                .css_classes(["line-bubble-text"])
                .build(),
        );
    }
    container
}

fn show(bubble: &gtk::Box, translation: &Translation) {
    remove_widget(bubble);
    let caption = crate::i18n::tf(
        "translate_direction",
        &[
            ("from", &translation.from.to_uppercase()),
            ("to", &translation.to.to_uppercase()),
        ],
    );
    bubble.append(&label_box(&caption, Some(&translation.text)));
}

fn bubble_for(id: &str) -> Option<gtk::Box> {
    BUBBLES.with(|b| b.borrow().get(id).and_then(|w| w.upgrade()))
}

fn request(state: &AppState, message_id: &str, text: &str, auto: bool) {
    let inserted = PENDING.with(|p| p.borrow_mut().insert(message_id.to_string()));
    if !inserted {
        return;
    }
    if !auto && let Some(bubble) = bubble_for(message_id) {
        remove_widget(&bubble);
        bubble.append(&label_box(&crate::i18n::t("translate_working"), None));
    }
    let (tx, rx) = async_channel::bounded(1);
    queue(job_for(state, text, auto), tx);
    let s = state.clone();
    let id = message_id.to_string();
    glib::spawn_future_local(async move {
        let result = rx
            .recv()
            .await
            .unwrap_or_else(|_| Err("translation worker stopped".into()));
        PENDING.with(|p| p.borrow_mut().remove(&id));
        let bubble = bubble_for(&id);
        match result {
            Ok(Outcome::Translated(translation)) => {
                if let Some(bubble) = &bubble {
                    show(bubble, &translation);
                }
                CACHE.with(|c| c.borrow_mut().insert(id, Entry::Shown(translation)));
            }
            Ok(Outcome::NotNeeded) => {
                if let Some(bubble) = &bubble {
                    remove_widget(bubble);
                }
                CACHE.with(|c| c.borrow_mut().insert(id, Entry::NotNeeded));
                if !auto {
                    toast(&s, &crate::i18n::t("translate_unchanged"));
                }
            }
            Err(error) => {
                if let Some(bubble) = &bubble {
                    remove_widget(bubble);
                }
                tracing::warn!(target: "line_gtk::translate", %error, auto, "translation failed");
                let first_auto_error = auto && !AUTO_ERROR_SHOWN.with(|f| f.replace(true));
                if !auto || first_auto_error {
                    toast(
                        &s,
                        &crate::i18n::tf("translate_failed", &[("error", &error)]),
                    );
                }
            }
        }
    });
}

fn auto_enabled(state: &AppState) -> bool {
    let Some(account) = state.self_mid.borrow().clone() else {
        return false;
    };
    let Some(chat) = state.current_chat.borrow().clone() else {
        return false;
    };
    state
        .config
        .borrow()
        .is_chat_auto_translated(&account, &chat)
}

/// Called for every text bubble as it is built (also when the virtual list
/// rebuilds a row): restore a shown translation or start auto-translation.
pub(super) fn attach(state: &AppState, bubble: &gtk::Box, message: &MessageInfo) {
    if !eligible(message) {
        return;
    }
    BUBBLES.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() > 4000 {
            b.retain(|_, w| w.upgrade().is_some());
        }
        b.insert(message.id.clone(), bubble.downgrade());
    });
    let cached = CACHE.with(|c| match c.borrow().get(&message.id) {
        Some(Entry::Shown(t)) => Some(Some(t.clone())),
        Some(_) => Some(None),
        None => None,
    });
    match cached {
        Some(Some(translation)) => show(bubble, &translation),
        Some(None) => {}
        None => {
            if !message.mine && auto_enabled(state) {
                request(state, &message.id, &message.text, true);
            }
        }
    }
}

/// Translate the incoming messages already on screen (after auto is switched on).
fn translate_visible(state: &AppState) {
    let ids: Vec<String> = BUBBLES.with(|b| {
        b.borrow()
            .iter()
            .filter(|(_, w)| w.upgrade().is_some())
            .map(|(id, _)| id.clone())
            .collect()
    });
    for id in ids {
        if CACHE.with(|c| c.borrow().contains_key(&id)) {
            continue;
        }
        let message = state.reply_messages.borrow().get(&id).cloned();
        if let Some(message) = message.filter(|m| !m.mine && eligible(m)) {
            request(state, &message.id, &message.text, true);
        }
    }
}

/// Forget translations, e.g. after the server or languages change.
pub(super) fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
    AUTO_ERROR_SHOWN.with(|f| f.set(false));
}

pub(super) fn append_menu(menu: &gio::Menu, message: &MessageInfo) {
    if !eligible(message) {
        return;
    }
    let item = gio::MenuItem::new(Some(&crate::i18n::t("translate_message")), None);
    item.set_action_and_target_value(
        Some("win.translate-message"),
        Some(&message.id.to_variant()),
    );
    menu.append_item(&item);
}

/// Check the configured server with a short Thai greeting; reports through `done`.
pub(super) fn test_server(config: &AppConfig, done: impl Fn(Result<String, String>) + 'static) {
    let job = Job {
        url: config.translate_url.clone(),
        api_key: config.translate_api_key.trim().to_string(),
        text: "สวัสดีครับ".into(),
        mine: if config.translate_language == "th" {
            config.translate_partner_language.clone()
        } else {
            config.translate_language.clone()
        },
        partner: "th".into(),
        auto: false,
    };
    let (tx, rx) = async_channel::bounded(1);
    queue(job, tx);
    glib::spawn_future_local(async move {
        let result = rx
            .recv()
            .await
            .unwrap_or_else(|_| Err("translation worker stopped".into()));
        done(match result {
            Ok(Outcome::Translated(t)) => Ok(t.text),
            Ok(Outcome::NotNeeded) => Ok("(unchanged)".into()),
            Err(error) => Err(error),
        });
    });
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("translate-message", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        let Some(id) = value.and_then(|value| value.str()) else {
            return;
        };
        let toggled = CACHE.with(|c| {
            let mut c = c.borrow_mut();
            match c.remove(id) {
                Some(Entry::Shown(t)) => {
                    if let Some(bubble) = bubble_for(id) {
                        remove_widget(&bubble);
                    }
                    c.insert(id.to_string(), Entry::Hidden(t));
                    true
                }
                Some(Entry::Hidden(t)) => {
                    if let Some(bubble) = bubble_for(id) {
                        show(&bubble, &t);
                    }
                    c.insert(id.to_string(), Entry::Shown(t));
                    true
                }
                // NotNeeded from auto mode: the user asked explicitly, so retry
                // allowing the reverse direction.
                Some(Entry::NotNeeded) | None => false,
            }
        });
        if toggled {
            return;
        }
        let text = s
            .reply_messages
            .borrow()
            .get(id)
            .filter(|m| eligible(m))
            .map(|m| m.text.clone());
        if let Some(text) = text {
            request(&s, id, &text, false);
        }
    });
    state.window.add_action(&action);

    let action =
        gio::SimpleAction::new("toggle-chat-auto-translate", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        let Some(mid) = value.and_then(|value| value.str()) else {
            return;
        };
        let Some(account) = s.self_mid.borrow().clone() else {
            return;
        };
        let on = !s.config.borrow().is_chat_auto_translated(&account, mid);
        let saved = s
            .config
            .borrow_mut()
            .set_chat_auto_translated(&s.data_dir, &account, mid, on);
        match saved {
            Ok(()) => {
                toast(
                    &s,
                    &crate::i18n::t(if on {
                        "auto_translate_enabled"
                    } else {
                        "auto_translate_disabled"
                    }),
                );
                if on && s.current_chat.borrow().as_deref() == Some(mid) {
                    AUTO_ERROR_SHOWN.with(|f| f.set(false));
                    translate_visible(&s);
                }
            }
            Err(error) => toast(
                &s,
                &crate::i18n::tf("translate_failed", &[("error", &error.to_string())]),
            ),
        }
    });
    state.window.add_action(&action);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thai_script_picks_the_direction_locally() {
        assert_eq!(
            first_direction("ไปกินข้าวกัน", "en", "th"),
            ("th".into(), "en".into())
        );
        assert_eq!(
            first_direction("see you at 7", "en", "th"),
            ("auto".into(), "en".into())
        );
        // A Thai reader gets Thai text translated into the partner language.
        assert_eq!(
            first_direction("ไปกินข้าวกัน", "th", "en"),
            ("th".into(), "en".into())
        );
        assert!(has_thai("ok ครับ"));
        assert!(!has_thai("hello"));
    }

    #[test]
    fn parses_libretranslate_responses() {
        let body = serde_json::json!({
            "translatedText": "Hello",
            "detectedLanguage": {"confidence": 90.0, "language": "th"}
        });
        assert_eq!(
            parse_reply(&body).unwrap(),
            ("Hello".into(), Some("th".into()))
        );
        let array = serde_json::json!({
            "translatedText": "Hi",
            "detectedLanguage": [{"confidence": 80.0, "language": "en"}]
        });
        assert_eq!(parse_reply(&array).unwrap().1, Some("en".into()));
        let plain = serde_json::json!({"translatedText": "Hi"});
        assert_eq!(parse_reply(&plain).unwrap(), ("Hi".into(), None));
        let error = serde_json::json!({"error": "Invalid API key"});
        assert_eq!(parse_reply(&error).unwrap_err(), "Invalid API key");
        assert!(parse_reply(&serde_json::json!({})).is_err());
    }

    /// Live check against a real server:
    /// `LINE_GTK_TRANSLATE_TEST_URL=http://127.0.0.1:5000 cargo test -- --ignored translate`
    #[test]
    #[ignore = "needs a LibreTranslate-compatible server"]
    fn translates_both_directions_against_a_live_server() {
        let Ok(url) = std::env::var("LINE_GTK_TRANSLATE_TEST_URL") else {
            return;
        };
        let job = |text: &str, auto: bool| Job {
            url: url.trim_end_matches('/').to_string(),
            api_key: String::new(),
            text: text.into(),
            mine: "en".into(),
            partner: "th".into(),
            auto,
        };
        let Outcome::Translated(thai) = run(&job("วันนี้อากาศดีมาก", true)).unwrap()
        else {
            panic!("Thai text should be translated");
        };
        assert_eq!((thai.from.as_str(), thai.to.as_str()), ("th", "en"));
        assert!(!has_thai(&thai.text), "{thai:?}");

        // Clicking an English message translates it into the partner language.
        let Outcome::Translated(english) =
            run(&job("Where shall we meet tonight?", false)).unwrap()
        else {
            panic!("English text should be translated on request");
        };
        assert_eq!((english.from.as_str(), english.to.as_str()), ("en", "th"));
        assert!(has_thai(&english.text), "{english:?}");

        // Auto mode leaves messages already in the reader's language alone.
        assert_eq!(
            run(&job("Where shall we meet tonight?", true)).unwrap(),
            Outcome::NotNeeded
        );
    }

    #[test]
    fn only_readable_text_is_offered_for_translation() {
        let message = |text: &str, content_type: &str| -> MessageInfo {
            serde_json::from_value(serde_json::json!({
                "id": "1", "text": text, "contentType": content_type,
            }))
            .unwrap()
        };
        assert!(eligible(&message("สวัสดี", "NONE")));
        assert!(!eligible(&message("[Image]", "IMAGE")));
        assert!(!eligible(&message("👍👍", "NONE")));
        assert!(!eligible(&message("", "NONE")));
    }
}
