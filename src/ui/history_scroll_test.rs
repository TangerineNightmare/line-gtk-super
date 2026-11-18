//! Frame-by-frame checks that the message viewport never flickers.
//!
//! Each sample is taken after a paint, so it is the geometry the user saw. Every
//! test owns the GTK main thread: run them one process at a time with
//! `scripts/test-gtk-fixtures.sh`.

use super::integration_tests::{fixture_state, message_json, settle};
use super::*;

#[derive(Clone, Debug)]
struct Seen {
    id: String,
    top: f32,
    bottom: f32,
}

type Frames = Vec<Vec<Seen>>;

struct Probe {
    frames: Rc<RefCell<Frames>>,
    active: Rc<Cell<bool>>,
    clock: gdk::FrameClock,
    handler: glib::SignalHandlerId,
}

fn visible_rows(state: &AppState) -> Vec<Seen> {
    let page = state.message_scroll.vadjustment().page_size() as f32;
    let mut rows: Vec<_> = state
        .message_rows
        .borrow()
        .iter()
        .filter(|(_, row)| row.is_mapped())
        .filter_map(|(id, row)| {
            let rect = row.compute_bounds(&state.message_scroll)?;
            (rect.y() + rect.height() > 0.0 && rect.y() < page).then(|| Seen {
                id: id.clone(),
                top: rect.y(),
                bottom: rect.y() + rect.height(),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.top.total_cmp(&b.top));
    rows
}

impl Probe {
    /// Sample after every paint, which is exactly the geometry on screen. A row
    /// added since the last layout has no position yet before that point.
    fn start(state: &AppState) -> Self {
        let frames = Rc::new(RefCell::new(Vec::new()));
        let active = Rc::new(Cell::new(true));
        let clock = state
            .message_scroll
            .frame_clock()
            .expect("the message list must be realized");
        let (s, frames_c, active_c) = (state.clone(), frames.clone(), active.clone());
        let handler = clock.connect_after_paint(move |_| {
            if active_c.get() {
                frames_c.borrow_mut().push(visible_rows(&s));
            }
        });
        // An idle clock paints nothing, so keep the frames coming.
        let active_c = active.clone();
        state.message_scroll.add_tick_callback(move |_, _| {
            if active_c.get() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        Self {
            frames,
            active,
            clock,
            handler,
        }
    }

    fn finish(self) -> Frames {
        self.active.set(false);
        self.clock.disconnect(self.handler);
        self.frames.take()
    }
}

/// Largest move of any row that was painted on two consecutive frames, against
/// the movement the reader asked for.
fn worst_step(frames: &Frames, expected: f32) -> (f32, usize, String) {
    let mut worst = (0.0_f32, 0, String::new());
    for (index, pair) in frames.windows(2).enumerate() {
        // A viewport that swaps all of its rows moved further than any row can show.
        if !pair[0].is_empty()
            && !pair[1].is_empty()
            && !pair[0]
                .iter()
                .any(|row| pair[1].iter().any(|n| n.id == row.id))
        {
            return (f32::INFINITY, index, "<no row survived>".into());
        }
        for row in &pair[0] {
            if let Some(next) = pair[1].iter().find(|other| other.id == row.id) {
                let error = (next.top - row.top - expected).abs();
                if error > worst.0 {
                    worst = (error, index, row.id.clone());
                }
            }
        }
    }
    worst
}

fn describe(frames: &Frames) -> String {
    frames
        .iter()
        .enumerate()
        .map(|(index, rows)| {
            let top = rows.first().map(|row| format!("{}@{:.0}", row.id, row.top));
            format!("{index}:{}", top.unwrap_or_else(|| "-".into()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rows of uneven height, like a real chat with quotes and long messages.
fn rich_json(id: u32) -> serde_json::Value {
    let mut message = message_json(id);
    match id % 4 {
        0 => {
            message["replyTo"] = serde_json::json!({
                "messageId": (id - 1).to_string(), "from": "u-friend",
                "text": "Trích dẫn một tin nhắn đã có trong bộ nhớ đệm", "contentType": "TEXT",
            });
        }
        1 => message["text"] = serde_json::json!(format!("Dài {id} ").repeat(90)),
        _ => {}
    }
    message
}

fn open_list(tag: &str, ids: std::ops::RangeInclusive<u32>) -> (AppState, std::path::PathBuf) {
    let (state, root) = fixture_state(tag);
    state.stack.set_visible_child_name("shell");
    *state.session_ready.borrow_mut() = true;
    set_msg_state(&state, "list", None);
    for id in ids {
        let message: MessageInfo = serde_json::from_value(rich_json(id)).unwrap();
        append_message(&state, &message, false);
    }
    reset_for_latest(&state, true);
    arm(&state);
    state.window.present();
    settle();
    let adj = state.message_scroll.vadjustment();
    assert!(adj.page_size() > 200.0, "fixture viewport is too small");
    (state, root)
}

fn close(state: AppState, root: std::path::PathBuf) {
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    let _ = std::fs::remove_dir_all(root);
}

fn page(ids: std::ops::Range<u32>) -> serde_json::Value {
    serde_json::json!({
        "messages": ids.map(rich_json).collect::<Vec<_>>(), "hasMore": true,
    })
}

fn scroll_to(state: &AppState, value: f64) {
    state.message_scroll.vadjustment().set_value(value);
    settle();
    state.message_scroll.vadjustment().set_value(value);
    settle();
}

fn request_older(state: &AppState) -> (u64, Pending) {
    state.history.borrow_mut().older_ready_at = None;
    state.history.borrow_mut().older_scroll_required = false;
    on_edge(state, Edge::Top);
    let id = state
        .history
        .borrow()
        .older_request
        .expect("a page must have been requested");
    let pending = state.pending.borrow_mut().remove(&id).unwrap();
    (id, pending)
}

/// Land a page with the reader idle and return every painted frame.
fn land_while_idle(state: &AppState, result: serde_json::Value) -> Frames {
    let (id, pending) = request_older(state);
    let probe = Probe::start(state);
    settle();
    response(state, id, pending, true, &result);
    settle();
    settle();
    probe.finish()
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

/// The target must appear once, fully visible, and then never move.
fn assert_arrives_once(tag: &str, frames: &Frames, target: &str, page_size: f32) {
    let arrival = frames
        .iter()
        .position(|rows| rows.iter().any(|row| row.id == target))
        .unwrap_or_else(|| panic!("{tag}: the target was never shown ({})", describe(frames)));
    let settled = &frames[arrival..];
    let (error, index, id) = worst_step(&settled.to_vec(), 0.0);
    assert!(
        error < 1.0,
        "{tag}: row {id} moved {error}px {index} frames after arriving ({})",
        describe(frames)
    );
    let row = settled[0].iter().find(|row| row.id == target).unwrap();
    assert!(
        row.top >= -1.0 && row.bottom <= page_size + 1.0,
        "{tag}: the target must be fully visible: {row:?} in {page_size}"
    );
}

fn assert_still(tag: &str, frames: &Frames) {
    let (error, index, id) = worst_step(frames, 0.0);
    assert!(
        error < 1.0,
        "{tag}: row {id} jumped {error}px at frame {index} ({})",
        describe(frames)
    );
    let (first, last) = (frames.first().unwrap(), frames.last().unwrap());
    for row in first {
        let end = last
            .iter()
            .find(|other| other.id == row.id)
            .unwrap_or_else(|| panic!("{tag}: row {} left the viewport", row.id));
        assert!(
            (end.top - row.top).abs() < 1.0,
            "{tag}: row {} drifted",
            row.id
        );
    }
}

#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_stay_still_when_older_page_lands_inside_the_list() {
    let (state, root) = open_list("scroll-mid", 301..=340);
    let page_size = state.message_scroll.vadjustment().page_size();
    scroll_to(&state, page_size);
    let frames = land_while_idle(&state, page(251..301));
    close(state, root);
    assert_still("mid-list", &frames);
}

/// At the very top GTK's scroll anchor is the loading row, which stays first
/// when a page lands, so without help the page pushes everything down.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_stay_still_when_older_page_lands_at_the_very_top() {
    let (state, root) = open_list("scroll-top", 301..=340);
    scroll_to(&state, 0.0);
    let frames = land_while_idle(&state, page(251..301));
    close(state, root);
    assert_still("top", &frames);
}

#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_stay_still_when_a_full_page_lands_and_the_far_end_is_trimmed() {
    let (state, root) = open_list("scroll-trim", 301..=500);
    let page_size = state.message_scroll.vadjustment().page_size();
    scroll_to(&state, page_size);
    let frames = land_while_idle(&state, page(251..301));
    assert_eq!(state.message_list.message_count(), WINDOW_MAX);
    close(state, root);
    assert_still("trim", &frames);
}

/// Momentum scrolling sets absolute adjustment positions, like GTK's kinetic
/// scroller, so a page landing mid-fling must not change the coordinates.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_keep_their_motion_when_a_page_arrives_during_a_fling() {
    const STEP: f32 = 5.0;
    const FLING: u32 = 40;
    let (state, root) = open_list("scroll-fling", 301..=380);
    let adj = state.message_scroll.vadjustment();
    let start = adj.page_size() * 4.0;
    scroll_to(&state, start);
    let (id, pending) = request_older(&state);
    let flung = Rc::new(Cell::new(0_u32));
    let landed = Rc::new(RefCell::new(Some((id, pending))));
    let (s, flung_c) = (state.clone(), flung.clone());
    state.message_scroll.add_tick_callback(move |_, _| {
        let frame = flung_c.get() + 1;
        flung_c.set(frame);
        if frame > FLING {
            return glib::ControlFlow::Break;
        }
        s.message_scroll
            .vadjustment()
            .set_value(start - f64::from(frame * STEP as u32));
        if frame == 12
            && let Some((id, pending)) = landed.borrow_mut().take()
        {
            response(&s, id, pending, true, &page(251..301));
        }
        glib::ControlFlow::Continue
    });
    let probe = Probe::start(&state);
    let deadline = Instant::now() + Duration::from_secs(4);
    while flung.get() <= FLING && Instant::now() < deadline {
        settle();
    }
    let frames = probe.finish();
    let moving = frames.len().min(FLING as usize - 4);
    let (error, index, row) = worst_step(&frames[..moving].to_vec(), STEP);

    // Once the fling is over the held page lands without moving anything.
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.history.borrow().older_request.is_some() && Instant::now() < deadline {
        settle();
    }
    let landed_page = state.message_list.message_count() > 80;
    let request_open = state.history.borrow().older_request.is_some();
    close(state, root);
    assert!(
        error < 2.0,
        "fling: row {row} was off by {error}px at frame {index} ({})",
        describe(&frames)
    );
    assert!(
        landed_page,
        "fling: the page must land after the scroll ends"
    );
    assert!(!request_open, "fling: the request must be finished");
}

/// Reply jumps start with keyboard focus inside the list, on the quote button.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_show_the_original_once_when_a_quote_is_clicked() {
    let (state, root) = open_list("scroll-quote", 301..=420);
    let mut quoting = rich_json(421);
    quoting["replyTo"] = serde_json::json!({
        "messageId": "330", "from": "u-friend",
        "text": "Tin nhắn gốc vẫn còn trong danh sách", "contentType": "TEXT",
    });
    let quoting: MessageInfo = serde_json::from_value(quoting).unwrap();
    append_message(&state, &quoting, false);
    pin_messages_to_latest(&state);
    settle();
    settle();
    let row = state.message_rows.borrow().get("421").unwrap().clone();
    let button = find(row.upcast_ref(), &|widget| {
        widget.has_css_class("line-reply-quote-button")
    })
    .and_downcast::<gtk::Button>()
    .expect("the reply row must show its quote");
    button.grab_focus();
    settle();
    let probe = Probe::start(&state);
    settle();
    button.emit_clicked();
    settle();
    settle();
    settle();
    settle();
    let frames = probe.finish();
    let page_size = state.message_scroll.vadjustment().page_size() as f32;
    close(state, root);
    assert_arrives_once("quote", &frames, "330", page_size);
}

/// A quoted message outside the loaded window arrives with an older page. The
/// page must not disturb the view before the single jump to the original.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_stay_still_until_a_paged_reply_target_arrives() {
    let (state, root) = open_list("scroll-paged", 301..=360);
    let adj = state.message_scroll.vadjustment();
    scroll_to(&state, (adj.upper() - adj.page_size()) * 0.5);
    let probe = Probe::start(&state);
    settle();
    jump_to_message(&state, "280");
    settle();
    let id = state
        .history
        .borrow()
        .older_request
        .expect("the missing original must be requested");
    let pending = state.pending.borrow_mut().remove(&id).unwrap();
    response(&state, id, pending, true, &page(251..301));
    settle();
    settle();
    settle();
    settle();
    let frames = probe.finish();
    let page_size = adj.page_size() as f32;
    close(state, root);
    let arrival = frames
        .iter()
        .position(|rows| rows.iter().any(|row| row.id == "280"))
        .unwrap_or_else(|| panic!("paged: never shown ({})", describe(&frames)));
    assert_still("paged, before arriving", &frames[..arrival].to_vec());
    assert_arrives_once("paged", &frames, "280", page_size);
}

/// A reload of cached rows that only changes a quote must leave the view alone.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_stay_still_when_a_reload_patches_a_visible_reply() {
    let (state, root) = fixture_state("scroll-patch");
    state.stack.set_visible_child_name("shell");
    *state.session_ready.borrow_mut() = true;
    set_msg_state(&state, "list", None);
    state.window.present();
    let ids = 301..=360;
    let messages = |quote: &str| -> Vec<MessageInfo> {
        ids.clone()
            .map(|id| {
                // Every row changes, so GTK's anchor row is always one of them.
                let mut message = message_json(id);
                message["replyTo"] = serde_json::json!({
                    "messageId": (id - 1).to_string(), "from": "u-friend",
                    "text": quote, "contentType": "TEXT",
                });
                serde_json::from_value(message).unwrap()
            })
            .collect()
    };
    apply_messages(&state, messages("trích dẫn cũ"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while state.render_batch.borrow().is_some() && Instant::now() < deadline {
        settle();
    }
    settle();
    let adj = state.message_scroll.vadjustment();
    scroll_to(&state, (adj.upper() - adj.page_size()) * 0.5);
    let shown = state.message_list.message_count();
    let probe = Probe::start(&state);
    settle();
    apply_messages(&state, messages("trích dẫn mới"));
    settle();
    settle();
    let frames = probe.finish();
    let count = state.message_list.message_count();
    close(state, root);
    assert_eq!(count, shown, "a patch must not rebuild the list");
    assert_still("patch", &frames);
}

/// The quoted message is already in the list, so the jump needs no paging and
/// must arrive in one step and then stop moving.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_show_a_cached_reply_target_once_and_hold_it() {
    let (state, root) = open_list("scroll-jump", 301..=420);
    let adj = state.message_scroll.vadjustment();
    scroll_to(&state, (adj.upper() - adj.page_size()) * 0.9);
    let probe = Probe::start(&state);
    settle();
    jump_to_message(&state, "330");
    settle();
    settle();
    settle();
    let frames = probe.finish();
    let page_size = adj.page_size() as f32;
    close(state, root);
    assert_arrives_once("jump", &frames, "330", page_size);
}

/// A live message scrolls the view once; it must never bounce.
#[test]
#[ignore = "needs a display; see scripts/test-gtk-fixtures.sh"]
fn frames_move_one_way_when_a_live_message_arrives_at_the_bottom() {
    let (state, root) = open_list("scroll-live", 301..=340);
    pin_messages_to_latest(&state);
    settle();
    settle();
    let probe = Probe::start(&state);
    settle();
    let message: MessageInfo = serde_json::from_value(rich_json(341)).unwrap();
    append_message(&state, &message, true);
    pin_messages_to_latest(&state);
    settle();
    settle();
    let frames = probe.finish();
    close(state, root);

    let track: Vec<f32> = frames
        .iter()
        .filter_map(|rows| rows.iter().find(|row| row.id == "340").map(|row| row.top))
        .collect();
    let backwards = track
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0_f32, f32::max);
    assert!(
        backwards < 1.0,
        "live: the view bounced back {backwards}px ({})",
        describe(&frames)
    );
}
