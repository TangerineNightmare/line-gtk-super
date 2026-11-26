# LTEngine for Line-GTK Super

[LTEngine](https://github.com/LibreTranslate/LTEngine) (AGPL-3.0, by the LibreTranslate
project) serves the LibreTranslate HTTP API using a local LLM (Gemma 3 through llama.cpp).
It handles casual Thai and mixed Thai/English much better than classic LibreTranslate.

Upstream only publishes CUDA container images. This directory has a CPU build of v0.1.1
and a podman Quadlet unit.

## Build the image

```bash
git clone --branch v0.1.1 --depth 1 https://github.com/LibreTranslate/LTEngine.git
podman build -t localhost/ltengine:v0.1.1-cpu \
  -f /path/to/line-gtk-super/packaging/ltengine/Containerfile LTEngine
```

The build compiles llama.cpp and takes several minutes.

## Run it

Quick test:

```bash
podman run --rm -p 127.0.0.1:5000:5050 -v ltengine-models:/models:Z \
  localhost/ltengine:v0.1.1-cpu -m gemma3-4b
```

As a user service that restarts automatically, use `ltengine.container` (instructions are
in the file).

The first start downloads the model from Hugging Face (`libretranslate/gemma3`, ~3 GB
for 4B Q4) into the `ltengine-models` volume. After that it starts in seconds.

Check it:

```bash
curl -s http://127.0.0.1:5000/translate -H 'Content-Type: application/json' \
  -d '{"q":"ขอบคุณมากนะครับ เจอกันพรุ่งนี้","source":"th","target":"en","format":"text"}'
# {"translatedText":"Thank you very much. See you tomorrow", ...}
```

A web UI is served at `/`.

## Performance

On a 12th-gen Intel laptop CPU with `gemma3-4b`, a typical chat message takes 3–4 s.
Line-GTK sends at most two requests at a time. A GPU build (see upstream's Dockerfile for
CUDA, or llama.cpp's Vulkan support) is much faster.
