# Fork notes

Line-GTK Super is a fork of [MidnightTale/Line-GTK](https://github.com/MidnightTale/Line-GTK).
The history is preserved: upstream commits keep their original authors, and this fork's
changes are separate commits on top.

## Upstream pull requests

At the time of the fork, upstream had these open pull requests, all based on upstream `4b85a0d`:

| PR | Author | Contents | Decision |
| --- | --- | --- | --- |
| [#12](https://github.com/MidnightTale/Line-GTK/pull/12) | WindyWin, with commits by shirafukayayoi | About 130 commits: reactions, quoted replies, mentions, unsend, read-receipt control, chat filters, pinned/hidden chats, albums, shared media, link previews, custom sticker library, message search, email/password login, Secret Service cache key, reconnect after sleep, bounded history, plain-LEGY default, Japanese and Vietnamese UI. Has CI. | **Base of this fork** (head `cc2a457`). |
| #4–#11 | yuki-kanemaru | Japanese UI, E2EE decryption fix (linejs 3.5.0), email login, chat-preview loading, call-history cards, read receipts, replies, category filter | **Already in #12.** |
| #1 | siraxuth | Reactions, unsend, receipts, multi-image, durable history, experimental video calls and Wayland screen sharing | **Not merged.** It conflicts with #12 in 24 core files and duplicates most of its features. The unique parts (video, screen sharing) are experimental. |
| #3 | nathawatc | AUR packaging using the system Deno | **Not merged.** Arch-only, and #12 rewrites the same PKGBUILDs. |

## Changes in this fork

| Commit subject | Why |
| --- | --- |
| Default UI and sidecar language to English | #12 defaults to Vietnamese |
| Allow LINE_GTK_LEGY_ENCRYPTED in the source-run sidecar | The compiled sidecar allowed the variable; source runs didn't |
| Add opt-in HTTP trace and escape toast text | Debugging login failures; toast text could break Pango markup |
| Remove unused food-poll modules | Dead code with a hard-coded street address and Google Maps/Places calls |
| Load link previews on click only | #12 fetched YouTube/TikTok/X previews as soon as a message was displayed |
| Update tests and formatting for English default and trace module | Tests follow the changes above |
| Translate messages through a LibreTranslate-compatible server | See [translation.md](translation.md) |

## LINE gateway note

Since about 18 November 2026, LINE's encrypted gateway `https://gf.line.naver.jp/enc`
returns empty 404s (see [linejs #280](https://github.com/evex-dev/linejs/issues/280)).
#12 already defaults to the plain LEGY endpoints (`/S4`, `/SYNC4` on
`legy.line-apps.com`, still HTTPS). `LINE_GTK_LEGY_ENCRYPTED=auto|true` switches back if
LINE fixes it. Letter Sealing message encryption is independent of this setting.

## Verification at fork time

- `cargo clippy --all-targets --all-features -- -D warnings`: clean.
- Rust tests: 190 passed. Display fixtures and the live translation test are ignored by default.
- `deno task --config protocol/deno.json check` (fmt, lint, type check, tests): 385 passed.
  #12 reports 390, and the difference is the 5 removed food-poll tests.
- The Ubuntu 24.04 tarball starts in a clean `ubuntu:24.04` container that has only the
  runtime packages from `packaging/ubuntu/INSTALL.md` installed.
- Used day to day on Ubuntu 24.04 for QR login, chat, and translation through LTEngine.
  The live translation test passes against LibreTranslate v1.9.6.

## Known limits

- QR login registers as an Android secondary device. A QR re-scan may be needed to fetch
  E2EE keys after upgrading from another build.
- The message cache is stored in plaintext unless a Secret Service provider and
  `secret-tool` (libsecret-tools) are available.
- Automatic sign-in at startup is off by default.
- #12 migrates the cache and settings format, so back up `~/.local/share/line-gtk`
  before switching from an older build.
