# Resonance

A DAW written in Rust: an [iced](https://iced.rs) app, a realtime audio
engine that hosts [CLAP](https://cleveraudio.org) plugins, a set of
first-party CLAP plugins (synth, drums, EQ, compressor, reverb, mastering,
…), and a control surface an AI client can drive over
[MCP](https://modelcontextprotocol.io).

It runs on **Linux** (PipeWire, Wayland) and **macOS** (CoreAudio, Cocoa).

- [Linux setup](#linux)
- [macOS setup](#macos)
- [Build and run](#build-and-run) (both platforms)
- [MCP server: drive the DAW from Claude](#mcp-server)
- [Troubleshooting](#troubleshooting)

For how the code is organised, see [ARCHITECTURE.md](ARCHITECTURE.md). For
tests and contributor conventions, see [CLAUDE.md](CLAUDE.md).

---

## Linux

### Requirements

- A **PipeWire** session. It's the default audio server on current Fedora,
  Ubuntu, Debian, and Arch installs. The engine talks to PipeWire directly
  and falls back to ALSA through cpal.
- A **Wayland** session (GNOME, KDE, Sway, Hyprland, …). The plugin editor
  windows are native Wayland windows.
- A GPU driver with **Vulkan** or OpenGL. The UI renders through wgpu.
- **Rust**, stable, installed with [rustup](https://rustup.rs).

### System packages

The PipeWire bindings are generated with bindgen, so they need clang. MP3
export compiles LAME from source, so it needs a C compiler. Opus export links
the system libopus. The file dialogs use GTK 3.

**Debian / Ubuntu**

```sh
sudo apt install build-essential clang pkg-config cmake \
  libpipewire-0.3-dev libspa-0.2-dev libasound2-dev libopus-dev \
  libgtk-3-dev libwayland-dev libxkbcommon-dev libegl-dev \
  mesa-vulkan-drivers
```

**Fedora**

```sh
sudo dnf install gcc gcc-c++ clang pkgconf-pkg-config cmake \
  pipewire-devel alsa-lib-devel opus-devel \
  gtk3-devel wayland-devel libxkbcommon-devel mesa-libEGL-devel \
  mesa-vulkan-drivers vulkan-loader
```

**Arch**

```sh
sudo pacman -S --needed base-devel clang pkgconf cmake \
  pipewire alsa-lib opus gtk3 wayland libxkbcommon mesa vulkan-icd-loader
```

### Realtime priority (recommended)

The engine renders on several threads. A worker thread is used only when it
can run at the same realtime priority as the audio thread. Otherwise the
engine renders serially, which works but is slower. To grant realtime
priority, add yourself to the `audio` group and allow it realtime scheduling:

```sh
sudo usermod -aG audio "$USER"
printf '@audio - rtprio 95\n@audio - memlock unlimited\n' \
  | sudo tee /etc/security/limits.d/95-audio.conf
```

Log out and back in, then check that `ulimit -r` prints `95`.

### Where plugins are found

The app scans `~/.clap`, `/usr/lib/clap`, every directory in `$CLAP_PATH`, and
the checkout's `target/bundled/`. You don't need to install the first-party
plugins to use them from a checkout.

---

## macOS

### Requirements

- macOS on Apple Silicon or Intel.
- **Xcode Command Line Tools**: `xcode-select --install`
- **Rust**, stable, installed with [rustup](https://rustup.rs).
- **Homebrew**, for three build dependencies:

```sh
brew install opus pkgconf cmake
```

The system `libopus` is required. Building opus from source fails with
CMake 4 (the bundled `audiopus_sys` build script is too old for it), and
`pkgconf` is how the build finds the Homebrew copy.

### Microphone permission

The first time Resonance touches an audio input, macOS asks for microphone
access. The app keeps working while the dialog is open. The input list, and
anything that records or monitors, waits until you answer. Resonance is
attributed to the app that launched it: your terminal, your IDE, or Claude
Code if an agent started it. To change the answer later, go to **System
Settings → Privacy & Security → Microphone**.

### Where plugins are found

The app scans `~/Library/Audio/Plug-Ins/CLAP`, `/Library/Audio/Plug-Ins/CLAP`
(including vendor subfolders), every directory in `$CLAP_PATH`, and the
checkout's `target/bundled/`. On macOS, `scripts/bundle.sh` produces real
`.clap` bundle directories with an `Info.plist`. To make them visible to other
hosts too, copy them into `~/Library/Audio/Plug-Ins/CLAP`.

---

## Build and run

The steps are the same on both platforms.

```sh
git clone <this repo> resonance && cd resonance

# 1. Build the first-party CLAP plugins into target/bundled/
./scripts/bundle.sh

# 2. Build and start the app
cargo run --release -p resonance-app
```

Skip step 1 and the app still starts, but it has no instruments. It logs a
warning that names `scripts/bundle.sh`. Re-run the script after changing any
plugin.

`.cargo/config.toml` builds with `-C target-cpu=native`. The binaries are
tuned for the machine that built them and aren't portable to older CPUs.

### Running the tests

```sh
./scripts/run-tests.py            # whole suite, in parallel
./scripts/run-tests.py -p <crate> # one crate
```

The golden images and bit-exact audio baselines were recorded on the Linux
x86-64 reference machine. On macOS, and on Apple Silicon in general, expect
those snapshot and hash tests to fail. The failures are platform baselines,
not regressions. [CLAUDE.md](CLAUDE.md) covers the rest: the grouped test
binaries, re-blessing goldens, and the live editor-window tests you run by
hand.

### Useful environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` | Log level, e.g. `RUST_LOG=info` or `RUST_LOG=resonance_audio=debug`. |
| `RESONANCE_NO_CONTROL=1` | Don't open the control socket (disables MCP control). |
| `RESONANCE_CONTROL_SOCKET` | Put the control socket at this path instead of the default. |
| `RESONANCE_RENDER_THREADS=<n>` | Number of render threads (1 = serial). |
| `RESONANCE_FORCE_CPAL_OUTPUT=1` | Linux: use cpal/ALSA output instead of native PipeWire. |
| `CLAP_PATH` | Extra plugin directories, `:`-separated. |

---

## MCP server

`resonance-mcp` lets Claude Code, Claude Desktop, or any MCP client drive a
**running** Resonance. It can create a project, write chords and parts, add
instruments and effects, mix, measure, and export. Every edit appears live in
the GUI and goes on the normal undo history.

The server is a thin stdio bridge. The app serves a JSON-RPC control socket,
and `resonance-mcp` connects to it and exposes each control method as an MCP
tool (`mcp__resonance__<tool>`).

```
Claude ──stdio──▶ resonance-mcp ──unix socket──▶ resonance-app
```

### 1. Build the server

```sh
cargo build --release -p resonance-mcp
# → target/release/resonance-mcp
```

### 2. Register it with Claude Code

```sh
claude mcp add --scope user --transport stdio resonance -- \
  "$PWD/target/release/resonance-mcp"
```

- Keep the name **`resonance`**. The tools are then named
  `mcp__resonance__<tool>`, which is what the skills in
  `resonance-agent-plugin/` and any permission allowlists expect.
- `--scope user` makes it available in every project. Leave it out to
  register it for the current directory only.
- Use an absolute path. Re-run `cargo build` after pulling so the server and
  the app come from the same checkout.

### 3. Start the app, then check the connection

```sh
cargo run --release -p resonance-app   # in another terminal
claude mcp list                        # expect: resonance ... ✔ Connected
```

In a Claude Code session, `/mcp` lists the tools. To check the connection,
ask Claude to call `control_hello`, which returns the app version and every
control method the running app implements.

The server starts fine when the app isn't running. Its tools then return
"resonance is not running — start the app and retry", and it reconnects on
the next call once the app is up.

### Claude Desktop (macOS)

Add this to `~/Library/Application Support/Claude/claude_desktop_config.json`
and restart Claude Desktop:

```json
{
  "mcpServers": {
    "resonance": {
      "command": "/absolute/path/to/resonance/target/release/resonance-mcp"
    }
  }
}
```

### Where the socket lives

Both sides compute the same path, so normally there's nothing to configure:

| Platform | Control socket |
|---|---|
| Linux | `$XDG_RUNTIME_DIR/resonance/control.sock` (usually `/run/user/<uid>/…`) |
| macOS | `<per-user temp dir>/resonance/control.sock` (under `/var/folders/…`) |
| Fallback | `/tmp/resonance-<uid>/control.sock` |

On macOS the per-user temp dir is asked of the OS
(`confstr(_CS_DARWIN_USER_TEMP_DIR)`, what `$TMPDIR` normally points at), not
read from the environment, so it holds even when Claude Desktop starts the MCP
server without `$TMPDIR` set. If the two sides still can't agree (e.g. a
sandboxed client with its own temp dir), give both the same path:

```sh
export RESONANCE_CONTROL_SOCKET="$HOME/.resonance-run/control.sock"   # for the app
claude mcp add --scope user --env RESONANCE_CONTROL_SOCKET="$HOME/.resonance-run/control.sock" \
  --transport stdio resonance -- "$PWD/target/release/resonance-mcp"
```

The socket's directory must be a real directory (not a symlink), owned by
you, with mode `0700`. The app creates it that way. Both sides refuse a
directory that isn't private, because anything that can write there could
feed Claude fake tool results. Keep the path short: on macOS a Unix socket
path has to be under 104 bytes.

### Optional: the agent skills

`resonance-agent-plugin/` is a Claude Code plugin of skills (song structure,
arranging, drumming, mixing, spatial, mastering). The skills teach Claude the
*procedure* for using the tools well. Load it with:

```sh
claude --plugin-dir "$PWD/resonance-agent-plugin"
```

It ships no MCP config of its own, so register the server as above first.
See [resonance-agent-plugin/README.md](resonance-agent-plugin/README.md) and
[resonance-mcp/README.md](resonance-mcp/README.md) for the full tool surface.

---

## Troubleshooting

**The plugin list is empty / no instruments.** Run `./scripts/bundle.sh`. The
app logs a warning that lists the directories it scanned.

**macOS: the input list is empty, or arming a track hangs.** A
microphone-permission dialog is waiting for an answer. It may be behind other
windows.

**macOS: `audiopus_sys` / CMake error while building.** Run
`brew install opus pkgconf`. The build must find the system libopus rather
than compile it from source.

**Linux: `libspa` / `pipewire` bindgen errors.** Install `clang` and the
PipeWire development headers. See [System packages](#system-packages).

**`claude mcp list` shows the server as failed.** Check the path passed to
`claude mcp add` exists and is executable, and run it directly to see its
error (`target/release/resonance-mcp` waits on stdin; press Ctrl-C to quit).

**Tools say "not running" while the app is open.** The app and the server are
resolving different socket paths. Set `RESONANCE_CONTROL_SOCKET` for both.
Also check the app wasn't started with `RESONANCE_NO_CONTROL=1`.

**Tools report `unsupported`.** The app binary is older than the MCP server.
Rebuild both from the same checkout and restart the app.
