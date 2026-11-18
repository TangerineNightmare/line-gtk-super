# Reply from notifications

M5a implemented on 2026-11-09. Desktop notifications offer **Reply**, opening a separate text composer with the destination conversation and source preview. Sending requires the Send button or Ctrl+Enter. Enter inserts a newline; Escape closes an idle composer. Opening or typing does not navigate the main conversation, change its draft, or mark messages read. Clicking the notification's ordinary Open action still opens its conversation under the existing read policy.

The reply quotes the original when the sidecar can resolve it from the current account's saved history and verify its conversation. A removed, undecryptable, expired, or unavailable source cannot silently become an unquoted message. The composer explains the problem and requires selecting **Send as a new message without quoting the original** before sending without the quote. A source unsend updates its composer even when a different conversation is open.

Send status appears in the composer. Confirmed success closes the composer without navigating the main window or changing its draft; this also applies to late confirmation after a timeout. Confirmed failures retain editable text for a deliberate retry. An uncertain result retains selectable text, disables sending, and asks the user to check the destination. No reply enters the reconnect outbox or automatically resends. Repeated actions reuse an existing composer, and repeated submissions cannot send again while pending, successful, or uncertain.

Incoming messages are grouped by account, protocol session, and conversation within a rolling 12-second window. Bursts wait 400 ms before display, or up to 1.2 seconds for missing sender/sticker details, using the first pending deadline so continuous traffic cannot postpone delivery indefinitely. The notification shows the last three previews, with sender names in groups, and Reply targets the newest message. Updates reuse the live notification and suppress repeat sounds. Dismissed native notifications start a fresh group. GIO fallback uses a stable per-conversation notification ID.

Offline text stays in the composer. If recovery replaces the protocol process, the old context becomes unusable and its text remains available to copy. Logout/login resets close the old composers. Drafts and notification contexts are memory-only and do not survive application restart.

## Action context and protocol

Notification actions carry a token resolved only on the GTK thread. Each context fixes the account, sidecar process generation, destination chat, source ID, preview, and creation time. Unopened contexts expire after two minutes; the registry retains at most 64, with at most eight open composers. New notifications replace the previous unopened context for the same conversation. Open composers keep their own immutable context. Stale actions display an explanation instead of sending.

Native notifications use the process's shared GIO D-Bus connection for creation, replacement, and action signals. The active desktop ID is kept separately from the bounded message-preview cache, so a still-live notification is replaced even after the 12-second grouping window or cache eviction. Replies use the ID returned by the notification server, including when that ID changes. Subscriptions accept signals only from that server's unique bus owner. Reply action IDs include their own token; the retained subscription forwards the token actually invoked. Native dismissal/action completion retires the unopened context. GIO fallback notifications use the same token-based application actions and are explicitly withdrawn on expiry/session invalidation because GIO does not report notification dismissal. A fresh application process cannot resolve a previous process's tokens.

The existing `send_message` RPC accepts optional `notificationReply: {accountMid, sourceMessageId, quote}` alongside `chatMid`, `text`, `replyToMessageId`, and M0's `operationId`. Source IDs remain strings. The sidecar rejects mismatched accounts/quote IDs and validates source ownership before preparing the send, including after disk loading. Session, contact-block and source state are checked again across send preparation. M0 continues to classify ambiguous transport outcomes and deduplicate operation IDs. Source lookup does not fetch history from LINE or issue read-receipt calls.

No new dependency or packaging permission is required. UI strings are provided in English, Japanese, Vietnamese, and Thai.

## Inline replies and desktop support

M5b remains disabled. The native GIO D-Bus adapter handles standard notification actions and close signals; it does not yet receive inline reply text. A daemon advertising a reply capability is insufficient without an adapter that receives and validates the text event. The supported fallback is the M5a composer.

| Path | Evidence | Still to verify |
| --- | --- | --- |
| Native notification Reply | Private D-Bus fixture invokes the replacement notification's new token through the retained listener and checks retirement | Real GNOME/KDE notification daemons, including background activation |
| GIO fallback | Token-based application action exercised by the GTK fixture; same context/session validation | Actual fallback rendering/dismissal on GNOME/KDE and Flatpak |
| GTK composer | Fixture checks destination/draft isolation, duplicate actions, offline text, source removal, explicit unquoted sends, failures, timeout/late confirmation, stale contexts and logout cleanup | Full physical keyboard/screen-reader pass and live LINE delivery |
| Inline notification text | Unavailable through the current Linux backend | A verified Linux adapter and native/Flatpak desktop matrix |

Fixtures use a fake sidecar, synthetic messages and a private notification service. They do not use credentials or send messages to LINE.

Screenshots: [light](screenshots/notification-reply-light.png), [dark, narrow window](screenshots/notification-reply-dark-narrow.png).

## Validation (2026-11-09)

- Rust formatting and Clippy with `-D warnings` passed. The ordinary Rust suite passed 179 tests, with 20 environment-dependent fixtures ignored.
- Both M5 fixtures passed separately with private D-Bus sessions. The GTK fixture also exercises the GIO application action and Ctrl+Enter handler, preserves the other chat's draft, inspects recorded RPC destinations/quote IDs, and checks that no mark-read request was issued. Light and dark screenshots at 460 px and 360 px were inspected.
- The Deno check task passed formatting, lint, type checking, and all 459 protocol tests. Notification regressions cover exact large source IDs, duplicate operations, account/chat mismatches, disk lookup/session changes, unsends and blocking during preparation, explicit unquoted sends, and uncertain outcomes.
- Offline sidecar compilation verified all 244 vendored files and passed ready/ping smoke checks using `/tmp/linegtk-m5-protocol`.
- `cargo build --offline --release` passed; no new dependency or packaging permission was introduced.

These checks cover the combined working tree. Live LINE delivery, a physical keyboard/screen-reader pass, and the GNOME/KDE native/Flatpak support matrix remain unverified.

## Follow-up validation (2026-11-10)

- Confirmed sends, including late confirmation after timeout, now close the composer. The GTK fixture verifies closure, context retirement, and preservation of the active conversation and draft.
- The GTK fixture uses a private notification daemon to verify four rapid messages produce one notification containing three previews, then a fifth message replaces its desktop ID. Unit coverage checks the grouping window and account/chat/process boundaries.
- Rust tests passed (184 passed, 30 environment-dependent fixtures ignored); the GTK reply/grouping fixture and native replacement/action-routing fixture passed separately. Formatting and Clippy with `-D warnings` passed. These remain fixture results, without live LINE or desktop-daemon compatibility claims.
- After a reported stacking issue, native delivery was moved to the persistent GIO D-Bus connection, and live notification IDs were separated from preview batching. The private daemon now verifies stable sender ownership, server-assigned replacement IDs, directed reply signals, and replacement after preview-cache eviction. The same Rust suite, both fixtures, and Clippy passed again.
