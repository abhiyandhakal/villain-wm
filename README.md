# Villain

Villain is a tiny Wayland compositor written in Rust with [Smithay](https://smithay.github.io/smithay/).

The name is temporary and the scope is intentional: this is a learning
compositor, not a desktop environment. The first milestone runs nested inside
the current desktop using Smithay's Winit backend. It creates a Wayland socket,
accepts clients, maps their XDG toplevel surfaces, and renders shared-memory
buffers into one output window.

## Run it

You need a Rust toolchain and the native packages required by Smithay's Winit
backend (`libwayland` and `libxkbcommon` on most Linux distributions).

```sh
cargo run
```

Villain prints the socket name it selected. In another shell, run a Wayland
client against that socket. For example, if it prints `wayland-1`:

```sh
WAYLAND_DISPLAY=wayland-1 weston-terminal
```

The client command must be started while Villain is running. Close the nested
window to stop the compositor.

## How to read this version

Start at [`src/main.rs`](src/main.rs), then follow this path:

1. `main` creates a `calloop` event loop, a Wayland `Display`, and `Villain`.
2. [`state.rs`](src/state.rs) registers the Wayland socket and adds the display
   as an event source. It also stores Smithay's protocol state.
3. [`handlers.rs`](src/handlers.rs) implements the protocol callbacks. A new
   XDG toplevel becomes a `Window` in the desktop `Space`.
4. [`render.rs`](src/render.rs) creates one nested output and renders the
   `Space` whenever the Winit window asks for a redraw.

The important boundary is that Wayland clients do not draw directly to our
window. They submit buffers to the compositor; the compositor decides where
and when those buffers become visible.

## Workspaces and input

Focus the nested Villain window and press `Alt+Enter`. Villain's keyboard
filter recognizes the combination and launches the executable named by
`VILLAIN_TERMINAL`, or `kitty` by default:

```sh
VILLAIN_TERMINAL=kitty cargo run
```

The important detail is that Villain sets `WAYLAND_DISPLAY` for the child
process to its own socket. The terminal therefore connects to Villain, and
its XDG toplevel becomes a window in Villain's `Space`. The shortcut is
intercepted, so it is not forwarded to a client.

There are ten workspaces, each holding one toplevel window. `Alt+1` through
`Alt+9` select workspaces 1–9; `Alt+0` selects 10. `Alt+Left` and `Alt+Right`
cycle with wraparound. An occupied workspace ignores `Alt+Enter`; repeated
launches are also blocked while the child is starting. Extra toplevels on an
occupied workspace receive a close request. Closing the app frees its slot.

Each app is configured fullscreen at the nested output size, including after
resizing. Ordinary keyboard input, pointer motion, clicks, and scrolling go
to the active app. The native Winit cursor supplies a visible arrow; custom
client cursor images are not implemented. Host desktop shortcuts can intercept
these combinations before Villain receives them.

[`workspaces.rs`](src/workspaces.rs) stores hidden windows while only the active
window is mapped into `Space`. Keyboard focus names its Wayland surface; pointer
focus additionally tracks the surface under the cursor. Smithay handles delivery
and pointer grabs during drags. [`keybinds.rs`](src/keybinds.rs) consumes both the
press and release of shortcut keys, even if Alt is released first.

Direct child process IDs associate launched terminals with the workspace where
they started. Use a standalone terminal executable for `VILLAIN_TERMINAL`, not
a wrapper or a single-instance launcher; unrelated external clients use the
current workspace. Child exits are reaped without blocking the event loop.
Wayland replies are flushed every loop iteration, independently of rendering.

To test: launch a terminal, type a command, select text with the mouse, then
switch to workspace 2 and launch another terminal. Switching back should restore
the first terminal. Test workspace 10, arrow-key wraparound, output resizing,
and closing a terminal with `exit` followed by opening it again.
Key diagnostics are available with `RUST_LOG=villain=debug cargo run`.

## Deliberate limitations

This version has no direct DRM backend, popups, clipboard, decorations,
custom cursor themes, or support for multiple windows within one workspace.
Those are separate learning steps. Keeping them out makes the event flow
visible and keeps the first compositor safe to run inside an existing session.

## Checks

```sh
cargo fmt --check
cargo check
```

The code follows Smithay 0.7.0's current public API. Smithay's own `smallvil`
example is a useful next comparison once this minimal path is understood.
