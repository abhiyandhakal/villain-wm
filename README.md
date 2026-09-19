# Villain

**Villain** is an experimental tiling-first Wayland compositor and window manager for the **Knave Desktop Environment**.

> *Wayland compositor* was once transcribed as *villain compositor*. The name stuck.

Villain is an attempt to build a compositor around a simple idea:

**A tiling desktop should feel like a complete desktop environment, not a collection of separately configured components.**

The goal is not to reproduce Hyprland, Sway, i3, GNOME, KDE, or any other existing environment. Villain will borrow ideas where they work, follow established Wayland protocols where possible, and develop its own window-management model where existing behavior does not fit the desktop Knave is trying to provide.

---

## Status

Villain is **very early experimental software**.

Expect missing functionality, broken behavior, changing APIs, and major architectural changes.

The first target is not feature completeness.

The first target is:

> **Become usable enough for its developer to run it.**

---

## Knave Desktop Environment

Villain is one part of the larger **Knave Desktop Environment**.

```text
Knave Desktop Environment
│
├── Villain
│   └── Wayland compositor + window manager
│
└── knaveshell
    ├── status bar
    ├── launcher
    ├── overview
    ├── notifications
    ├── quick settings
    └── other desktop UI
```

Villain and `knaveshell` are separate components, but together form one desktop experience.

The separation is intentional:

* **Villain** owns windows, workspaces, input, layout, focus, activation, outputs, and composition.
* **knaveshell** owns desktop-facing UI.
* The user should normally think about **Knave**, not about which internal component implements a particular feature.

---

## Design Philosophy

### Tiling first

Tiling is not an optional mode layered on top of a traditional floating desktop.

Villain assumes from the beginning that windows normally participate in layouts.

Floating windows will exist where they make sense, but the desktop itself is designed around tiling.

---

### Usable by default

The user should not have to assemble a desktop from:

* a compositor,
* a bar,
* a launcher,
* a notification daemon,
* a lock screen,
* scripts,
* several unrelated configuration files,
* and enough glue to keep them cooperating.

Knave should provide a coherent working environment out of the box.

---

### Opinionated, but customizable

Villain should expose **meaningful choices**, not every internal implementation detail.

Customization should allow users to change the desktop without making ordinary configurations unpredictable or internally inconsistent.

The intention is roughly:

```text
strong defaults
      +
useful customization
      +
advanced escape hatches
      -
configuration for configuration's sake
```

There should still be a recognizable answer to:

> “How does Villain behave?”

even on a customized installation.

---

### Window semantics matter

Window management is more than deciding where rectangles go.

Villain intends to treat concepts such as these as first-class behavior:

* tiled windows
* floating windows
* fullscreen windows
* minimized or shelved windows
* workspaces
* activation
* focus
* attention requests
* transient windows
* popups

These states should have clear semantics rather than being approximated through unrelated mechanisms.

---

## Planned Window Behaviour

### Minimization / shelving

Tiling should not mean that every open application must permanently consume layout space.

A minimized window remains associated with its workspace but temporarily stops participating in its visible layout.

```text
Before:

┌──────────────┬──────────────┐
│              │              │
│   Terminal   │   Browser    │
│              │              │
├──────────────┴──────────────┤
│           Editor            │
└─────────────────────────────┘


Browser minimized:

┌─────────────────────────────┐
│          Terminal           │
├─────────────────────────────┤
│           Editor            │
└─────────────────────────────┘

Hidden:
[ Browser ]
```

Moving a window to another workspace and minimizing it are different operations.

Workspaces represent context.

Minimization represents visibility.

---

### Activation

Villain should distinguish between:

* explicit user-driven activation,
* application-generated attention requests,
* and unwanted background focus stealing.

For example:

```text
User clicks "Open in application"
        ↓
target exists on another workspace
        ↓
switch workspace
        ↓
focus target
```

But:

```text
background application requests attention
        ↓
mark as requiring attention
        ↓
do not unexpectedly steal focus
```

Activation policy should be deliberate rather than reduced to a single global `true` / `false` switch.

---

### Predictable layouts

Opening applications should produce sensible layouts without requiring the user to manually construct a container tree.

For example:

```text
1 window

┌───────────────────────┐
│           A           │
└───────────────────────┘
```

```text
2 windows

┌───────────┬───────────┐
│     A     │     B     │
└───────────┴───────────┘
```

```text
3 windows

┌───────────────┬───────┐
│               │   B   │
│       A       ├───────┤
│               │   C   │
└───────────────┴───────┘
```

The current implementation uses a 50/50 master-and-stack layout. Windows keep
their creation order: the first visible window is the master and later windows
append to the stack. A minimized window keeps its place in that order but is
left out of the visible layout until restored.

Child dialogs and fixed-size windows float above the tiles. Dialogs belong to
their parent's workspace and start centered over it; standalone fixed-size
windows start centered on the output. Floating windows honor client size limits
and stay within the output. Applications can initiate title-bar moves and edge
resizes while a pointer button is held on their window.

App-requested fullscreen fills the current output and temporarily hides the
workspace's other windows, except the fullscreen app's child dialogs. Exiting
fullscreen restores the tiled layout or the floating window's previous geometry.
A fullscreen request on an inactive workspace does not switch workspaces.
Minimizing a fullscreen window reveals the workspace; restoring it restores
fullscreen. Explicitly focusing a window hidden behind fullscreen exits fullscreen.
These behaviors apply to both native Wayland and XWayland applications.

`villainctl windows` reports `floating` and `fullscreen` alongside the existing
window state. `floating` describes the window's normal layout, including while
it is temporarily fullscreen.

### Desktop layers

Villain supports `wlr-layer-shell` on both the nested and TTY backends. Launchers
such as Wofi can use their native layer-shell mode (`wofi --show drun`), and
panels, notifications, and wallpapers belong to the output across workspace
switches. These surfaces are excluded from ordinary window lists and tiling.

The background and bottom layers render below application windows; top and
overlay render above them, including fullscreen applications. Anchors, margins,
requested sizes, and exclusive zones control placement. Panels with an exclusive
zone reserve space for tiled windows; fullscreen windows still fill the output.

Keyboard interactivity is honored: noninteractive layers do not take keyboard
focus, on-demand layers follow Villain's pointer focus policy, and exclusive
layers on top/overlay retain keyboard focus until dismissed. Layer popups are
configured, rendered, and hit-tested, including outside their parent's bounds;
valid pointer-initiated popup grabs survive workspace switches. Unmapping or
closing a layer releases its reserved space and popup grabs. Layer surfaces and
their popups receive paced frame callbacks through the shared frame clock.

### Current controls

The temporary mod key is `Alt` while Villain is being tested.

| Binding | Action |
| --- | --- |
| `Alt+Enter` | Open a terminal |
| `Alt+Q` | Close the focused window |
| `Alt+M` | Minimize the focused window |
| `Alt+Shift+M` | Restore the last minimized window |
| `Alt+1`–`Alt+9`, `Alt+0` | Select workspace 1–10 |
| `Alt+Left`, `Alt+Right` | Select the previous or next workspace |

Application keyboard focus is click-to-focus; pointer motion alone does not move
focus. Explicit compositor actions such as creating a window, switching workspaces,
or closing a window take precedence over the pointer position.

Villain renders its own cursor in both nested and direct modes. It follows
client-provided cursor surfaces and the standard cursor-shape protocol, loading
named shapes from `XCURSOR_THEME` at `XCURSOR_SIZE` when those variables are
set. The host compositor's cursor is hidden while it is over the nested output.

Clipboard state is local to Villain, including when it runs nested inside
another compositor. Standard clipboard, primary selection, and data-control
protocols are available so ordinary applications and clipboard managers can
exchange selections without leaking them into the host session.

Villain starts XWayland as an optional compatibility subsystem. X11 windows
use the same workspace, layout, focus, close, and minimize model as native
Wayland windows, while override-redirect surfaces remain unmanaged. Clipboard
and primary selections are bridged in both directions. `DISPLAY` is only
exported after XWayland reports that it is ready; native Wayland operation
continues if XWayland is unavailable.

### Configuration

Villain loads `~/.config/villain/config.toml` at startup. The built-in defaults
use Super as `MOD`, open the Knave workspace overview when `MOD` is pressed and
released on its own, enable touchpad tapping and natural scrolling, and preserve
the close, minimize, terminal, and workspace bindings. See `config.example.toml`
for the complete format. Modifier combinations such as `MOD+1` cancel the
standalone overview action.

Session variables come from `~/.config/villain/environment` by default, or the
file selected by `environment_file`. It accepts literal `KEY=VALUE` and
`export KEY=VALUE` lines; it does not execute shell syntax or expand `$HOME`.
Villain supplies Wayland-native defaults for XDG, Electron, Mozilla, Qt, and
GTK applications. These values are inherited by applications spawned after a
reload, including descendants of a newly opened terminal.

Configuration reload is atomic:

```console
villainctl reload
```

Villain parses and validates the entire replacement before changing runtime
state. A successful reload replaces the keybind registry, reapplies touchpad
settings to connected devices, and updates the environment for future spawned
applications. Existing application processes retain their original environment.

On the direct TTY backend, Villain publishes its display and desktop variables
to the systemd user manager and D-Bus activation environment. This lets
`xdg-desktop-portal` and its GTK backend connect to the Villain session. The
shipped `share/xdg-desktop-portal/villain-portals.conf` selects GTK for the
generic portals it implements, including file choosers, notifications,
printing, and settings. Packaged builds should install that file below their
matching `share` prefix. Nested development mode deliberately leaves the host
desktop's activation environment alone.

Screen capture is a separate portal backend concern. Villain does not claim
Hyprland's portal backend: screenshot, screencast, remote-desktop, and global
shortcut portals remain unavailable until Villain provides the corresponding
capture protocols and backend.

If `config.toml` contains any `[[bind]]` entries, they replace the complete
default binding set. `MOD` follows `modkey`; explicit `ALT`, `SUPER`, `CTRL`,
and `SHIFT` modifiers remain available.

---

## Architecture

Villain conceptually separates compositor mechanism from window-management policy.

Keyboard shortcuts and external clients share one imperative dispatcher. IPC
queries inspect state directly; they are not dispatcher actions.

```text
keyboard -> keybind matching --+
                              +-> dispatcher -> compositor state
villainctl -> IPC dispatch ---+
villainctl -> IPC query ----------------------> compositor state
```

The Cargo workspace currently contains three packages:

* `villain` — the compositor, dispatcher, window state, and IPC server
* `villain-ipc` — Smithay-independent serializable types and client code
* `villainctl` — a thin command-line IPC client

```text
Wayland clients
      │
      ▼
┌─────────────────────────────┐
│           Villain           │
│                             │
│  Wayland protocol handling  │
│  input                      │
│  rendering                  │
│  outputs                    │
│                             │
│  ─────────────────────────  │
│                             │
│  window model               │
│  layouts                    │
│  workspaces                 │
│  focus                      │
│  activation                 │
│  minimization               │
└──────────────┬──────────────┘
               │
               ▼
            displays
```

A normal application window will generally enter Villain through an `xdg_toplevel`.

Villain then maintains its own higher-level representation of that window:

```text
xdg_toplevel
      ↓
Villain window
      ↓
workspace
      ↓
layout policy
      ↓
geometry
      ↓
scene / renderer
```

The compositor mechanism should not dictate the desktop's window-management policy.

---

## Villain and knaveshell

`knaveshell` will run separately from Villain rather than being embedded directly into the compositor process.

Conceptually:

```text
┌─────────────────────────┐
│       knaveshell        │
│                         │
│ bar                     │
│ overview                │
│ launcher                │
│ notifications           │
│ quick settings          │
└────────────┬────────────┘
             │
       Wayland + IPC
             │
┌────────────▼────────────┐
│         Villain         │
│                         │
│ compositor              │
│ window manager          │
└─────────────────────────┘
```

Shell surfaces such as panels, launchers, and overlays are not normal application windows and should not participate in ordinary tiling layouts.

Villain exposes its window-management state and actions to `knaveshell` through
the IPC interface below. Wayland protocols remain the interface for ordinary
client and shell-surface behavior.

### IPC and `villainctl`

Villain listens on a user-only Unix socket scoped to its Wayland display:

```text
$XDG_RUNTIME_DIR/villain-$WAYLAND_DISPLAY.sock
```

Requests and responses are JSON Lines. Workspace numbers at the IPC boundary
are one-based. Window IDs are monotonic for the lifetime of the compositor and
are not reused.

Initial commands include:

```console
villainctl dispatch workspace 2
villainctl dispatch minimize
villainctl dispatch focus-window 1
villainctl dispatch restore-window 1
villainctl dispatch exec kitty

villainctl windows
villainctl workspaces
villainctl active-window
villainctl active-workspace
villainctl version
```

Protocol version 2 also exposes an on-demand `workspace-preview` query for the
shell. It returns a bounded, base64-encoded PNG rendered from the workspace's
current client buffers. Preview requests are limited to 64x36 through 1280x720
so a local client cannot force unbounded compositor allocations. Layer-shell
surfaces and the cursor are intentionally excluded from workspace previews.

`villainctl` discovers the compositor through `WAYLAND_DISPLAY`. Set
`VILLAIN_SOCKET` only when an explicit socket override is needed.

---

## Existing Ecosystem

Villain does not intend to reimplement every component of a Linux desktop.

It should reuse standard Linux and Wayland infrastructure wherever practical.

Likely integration areas include:

* Wayland protocols
* PipeWire
* XDG Desktop Portals
* libinput
* DRM/KMS
* systemd
* polkit
* NetworkManager
* existing application ecosystems

Hyprland and other compositors are useful references for protocol support, hardware quirks, NVIDIA compatibility, ecosystem integration, and accumulated lessons from real-world compositor development.

Villain may intentionally support compatible protocols where doing so makes existing tools reusable.

However, Hyprland-specific architecture should not become Villain's internal architecture.

---

## Technology

Villain is written in **Rust** and built on **Smithay**.

Smithay provides the low-level building blocks for the Wayland compositor, including protocol handling, backend integration, input, rendering, outputs, and compositor infrastructure.

Villain builds its own desktop and window-management policy on top of those primitives.

Conceptually:

```text
Linux / Wayland / DRM / input
              │
           Smithay
              │
           Villain
      ┌───────┴────────┐
      │                │
 window management   compositor policy
      │                │
      └───────┬────────┘
              │
       Knave Desktop
```

Using Smithay avoids reimplementing generic compositor infrastructure while keeping Villain's window model, layouts, workspace semantics, activation policy, and other desktop behavior under its own control.

The project may still change substantially while the compositor is developed, and no internal or public API should currently be considered stable.

---

## Why?

Because tiling window managers are good at managing windows.

Traditional desktop environments are good at being complete desktops.

There is still interesting design space in treating **tiling itself as the foundation of a complete desktop environment** rather than as an optional mode or a system the user must assemble themselves.

And because building a Wayland compositor sounds fun.

---

## License

MIT
