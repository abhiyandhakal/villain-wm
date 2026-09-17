# Villain

**Villain** is an experimental tiling-first Wayland compositor and window manager for the **Abhi Desktop Environment**.

> *Wayland compositor* was once transcribed as *villain compositor*. The name stuck.

Villain is an attempt to build a compositor around a simple idea:

**A tiling desktop should feel like a complete desktop environment, not a collection of separately configured components.**

The goal is not to reproduce Hyprland, Sway, i3, GNOME, KDE, or any other existing environment. Villain will borrow ideas where they work, follow established Wayland protocols where possible, and develop its own window-management model where existing behavior does not fit the desktop Abhi is trying to provide.

---

## Status

Villain is **very early experimental software**.

Expect missing functionality, broken behavior, changing APIs, and major architectural changes.

The first target is not feature completeness.

The first target is:

> **Become usable enough for its developer to run it.**

---

## Abhi Desktop Environment

Villain is one part of the larger **Abhi Desktop Environment**.

```text
Abhi Desktop Environment
│
├── Villain
│   └── Wayland compositor + window manager
│
└── abhishell
    ├── status bar
    ├── launcher
    ├── overview
    ├── notifications
    ├── quick settings
    └── other desktop UI
```

Villain and `abhishell` are separate components, but together form one desktop experience.

The separation is intentional:

* **Villain** owns windows, workspaces, input, layout, focus, activation, outputs, and composition.
* **abhishell** owns desktop-facing UI.
* The user should normally think about **Abhi**, not about which internal component implements a particular feature.

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

Abhi should provide a coherent working environment out of the box.

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

Keyboard focus follows the pointer across visible windows.

Villain renders its own cursor in both nested and direct modes. It follows
client-provided cursor surfaces and the standard cursor-shape protocol, loading
named shapes from `XCURSOR_THEME` at `XCURSOR_SIZE` when those variables are
set. The host compositor's cursor is hidden while it is over the nested output.

Clipboard state is local to Villain, including when it runs nested inside
another compositor. Standard clipboard, primary selection, and data-control
protocols are available so ordinary applications and clipboard managers can
exchange selections without leaking them into the host session.

---

## Architecture

Villain conceptually separates compositor mechanism from window-management policy.

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

## Villain and abhishell

`abhishell` will run separately from Villain rather than being embedded directly into the compositor process.

Conceptually:

```text
┌─────────────────────────┐
│       abhishell         │
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

Villain and `abhishell` may eventually use a private protocol or IPC interface for functionality that cannot appropriately be exposed to arbitrary Wayland clients.

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
       Abhi Desktop
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
