# Line-GTK Super on Ubuntu

Line-GTK Super is an unofficial LINE client and a fork of
[MidnightTale/Line-GTK](https://github.com/MidnightTale/Line-GTK). This tarball is x86_64,
built in a podman `ubuntu:24.04` container by `packaging/ubuntu/build.sh`.

## Building the tarball

On any Linux host with podman (or `CONTAINER_ENGINE=docker`):

    ./packaging/ubuntu/build.sh          # Ubuntu 24.04
    ./packaging/ubuntu/build.sh 26.04    # Ubuntu 26.04

The output is `target/dist/line-gtk-super-<version>-<sha>-ubuntu<rel>-x86_64.tar.gz`.
The script builds the committed HEAD and runs the Rust and Deno test suites.

## Required packages (Ubuntu 24.04; also fine on 26.04)

    sudo apt-get install --no-install-recommends libgtk-4-1 libadwaita-1-0 libdbus-1-3 libssl3t64

Optional:

    sudo apt-get install --no-install-recommends ffmpeg poppler-utils pulseaudio-utils xdg-utils libsecret-tools

- `ffmpeg`: voice messages, video thumbnails, custom sticker import and optimisation.
- `poppler-utils`: PDF previews.
- `libsecret-tools` (`secret-tool`): keeps the message-cache key in GNOME Keyring /
  Secret Service instead of a plain file.

## Run

    ./line-gtk

The data dir is `~/.local/share/line-gtk/`, shared with upstream Line-GTK.

**If you used another Line-GTK build before, back it up before first run.** This
version migrates the cache and settings format, and older builds may not read them
afterwards.

## Environment switches

| Variable | Effect |
|---|---|
| `LINE_GTK_LEGY_ENCRYPTED=false` (default) / `auto` / `true` | LEGY transport. Plain is the workaround for LINE's `/enc` 404s (linejs #280). |
| `LINE_GTK_TRACE_HTTP=1` | Redacted HTTP trace to `line-gtk.log` |
| `RUST_LOG=debug` | Verbose app log |
| `LINE_VERSION=26.18.2` | Override the Android version reported to LINE |

## Message translation

Right-click a message → **Translate** (click again to hide). Chat settings →
**Auto-translate this chat** translates incoming messages that aren't in your
language. Configure in Settings → Features → Translation (server URL, optional
API key, "Translate into" = `en`, "Other language" = `th`, Test connection).

The client speaks the LibreTranslate HTTP API (`POST /translate`). Default server:
a local LibreTranslate container, with only English and Thai loaded:

    podman run -d --name libretranslate --restart=unless-stopped \
      -p 127.0.0.1:5000:5000 -v libretranslate-models:/home/libretranslate/.local \
      docker.io/libretranslate/libretranslate:v1.9.6 --load-only en,th

Add more languages with e.g. `--load-only en,th,ja,zh`.

For better quality with slang and mixed Thai/English, point the URL at another
server that speaks the same API:
- **LTEngine** (LibreTranslate project, local LLM via llama.cpp, Gemma 3;
  default 4B model ~4 GB RAM): https://github.com/LibreTranslate/LTEngine. The
  repository has a CPU container and systemd unit in `packaging/ltengine/`.
- A hosted LibreTranslate instance (e.g. libretranslate.com, which needs an API key).

Message text is sent to whichever server is configured. That includes the
decrypted text of end-to-end-encrypted chats, so keep it local or trusted.

More detail: `docs/translation.md` in the source repository.
