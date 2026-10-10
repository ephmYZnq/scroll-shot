# scroll-shot

<div align="center">

**Ultra-lightweight, high-resolution, pixel-perfect scrolling screenshot and pinning tool designed for Linux desktop.**

[English](README.md) | [简体中文](README_zh.md)

</div>

---

`scroll-shot` is a native system screenshot and scrolling capture (long screenshot) utility written in pure Rust. It is engineered specifically for Linux desktop environments, delivering **instant cold startup, low memory footprint, a single standalone binary (<2MB), zero heavy GUI/framework dependencies**, and **100% 1:1 pixel-lossless stitching quality**.

---

## 🌟 Key Features

### 1. 📋 Direct System Clipboard Integration (Instant Paste, No Disk Clutter)
- Captured images (standard shots, annotated images, or long stitched screenshots) are **automatically written to the X11 clipboard** upon completion.
- Switch to chat apps (Slack, Discord, Telegram, WeChat), emails, or documents and paste directly with **`Ctrl+V`**!
- If file archiving is desired, explicitly save with `-o <PATH>`.

### 2. 📌 Pin to Desktop
- Floating desktop pin window inspired by Snipaste / PixPin:
  - **Fluid Window Dragging**: Drag from anywhere within the window. Optimized with X11 motion event coalescing for buttery smooth 60~144Hz movement even on 4K HiDPI displays.
  - **Lossless Zoom**: Mouse wheel scaling from 0.15x to 4.0x with nearest-neighbor and bilinear filtering. Press `0` to instantly restore 1:1 scale.
  - **Window Opacity Control**: Hold `Ctrl` or `Shift` + mouse wheel to adjust transparency smoothly (30% to 100% alpha).
  - **Quick Dismiss**: Double click, press `ESC`, or click the top-right button to close.

### 3. 🎨 Advanced Annotator
- **Step Counters ①②③** (Hotkey `n`): Click to place auto-incrementing step bubbles to clarify user guides and bug reproduction steps. Supports `Ctrl+Z` undo to rewind counters.
- **Interactive In-Place Typing T Text** (Hotkey `t`): Type text directly onto screenshots with cursor navigation, backspace editing, and enter to commit. Text is rendered with a 1px protective drop shadow for high contrast.
- **Classic Drawing Tools**: Rectangles, Arrows, Mosaic/Blur brush, Highlighter strokes, and 4-color palette.

### 4. 🔤 Offline Multi-Language OCR & Interactive Text Selection
- **Instant Offline Recognition**: Click **`OCR`** on the toolbar (or press `o`) to extract code snippets, error logs, or paragraphs in seconds.
- **Bilingual & Multi-Language Support**: Automatically queries and combines installed Tesseract language models (e.g. `chi_sim`, `chi_tra`, `eng`, etc.) for seamless mixed-language recognition.
- **FreeType Antialiased Typography**: Dynamically binds to Linux system FreeType (`libfreetype.so.6`) to render authentic CJK and Latin vector glyphs with zero garbled symbols.
- **Interactive Mouse Drag Selection & Selection-Only Copy**:
  - Drag the mouse across single or multiple lines with royal blue highlight rendering.
  - **Smart Copying**: Press `Ctrl+C`, `c`, or click the copy button to **copy ONLY highlighted text** when a selection is active; copies full text when nothing is selected.
  - Press `Ctrl+A` to select all text.
  - Automatically copies text to both X11 `CLIPBOARD` (`Ctrl+V`) and `PRIMARY` (middle-click paste).

### 5. 🎯 Native Pixel Icons & Polished UI
- All toolbar buttons feature custom 8x8 native pixel icons (Camera, Pin, Pencil, OCR scanner, Scroll arrows, Close cross).
- Independent of system Emoji fonts, eliminating missing glyph boxes (`☒`) and ensuring a razor-sharp, consistent look across all Linux distributions.

### 6. 📜 Human-Driven Live Scrolling Capture (No Stiff Auto-Scrolling)
- Replaces rigid robotic auto-scrolling with intuitive human-guided capture:
  - While scrolling, a sleek height recorder bar appears at the top, accompanied by a **live proportional thumbnail preview** on the right.
  - Scroll naturally using **mouse wheel, trackpad gestures, or by dragging the scrollbar**. The engine captures incoming frames at high frame rates and stitches them with sub-pixel alignment.
  - When finished, click **`[Done]`** on the toolbar (or hit `Enter`) to finalize and copy directly to your clipboard.

### 7. 🖥️ Fullscreen Freeze Mask & Global Hotkey
- Launches with an instant dark freeze mask modeled after native desktop screen capture tools.
- Drag to select region, resize with 8-direction handles, or move selection box.
- **Global Hotkey [Ctrl + 1]**: Instant summons anywhere on your desktop—ideal for full-screen workflows and virtual machine operations.

---

## 📐 Processing Pipeline

```
┌──────────────────┐     ┌───────────────────┐     ┌───────────────────────┐
│ 1. Area Selector │ ──> │ 2. Capture Loop   │ ──> │ 3. 1D Lossless Stitch │
│ Mouse drag-box / │     │ XShm grabs frame  │     │ Extract overlap slice │
│ CLI coordinates  │     │ User scrolls page │     │ Detect vertical Δy    │
└──────────────────┘     └───────────────────┘     └──────────┬────────────┘
                                                              │
                                                              ▼
                                                   ┌───────────────────────┐
                                                   │ 4. Live Preview & Out │
                                                   │ Dynamic canvas append │
                                                   │ Copy to clipboard     │
                                                   └───────────────────────┘
```

---

## 🚀 Quick Start

### 1. Requirements
- **OS**: Linux (X11 desktop: GNOME, KDE, XFCE, etc.).
- **Rust Toolchain**: Rust 1.75+ (`cargo`).
- **Optional: Local Offline OCR Component**:
  - `scroll-shot` uses standard pipelines to interface with the lightweight `tesseract-ocr` engine:
    - **Ubuntu / Debian / Linux Mint / Deepin**:
      ```bash
      sudo apt install -y tesseract-ocr tesseract-ocr-chi-sim
      ```
    - **Arch Linux / Manjaro**:
      ```bash
      sudo pacman -S tesseract tesseract-data-chi_sim tesseract-data-eng
      ```
    - **Fedora / RHEL**:
      ```bash
      sudo dnf install -y tesseract tesseract-langpack-chi_sim tesseract-langpack-eng
      ```
  - *Note: If not installed, clicking OCR displays a friendly installation command with a one-click `[Copy Command]` button.*

### 2. Build & Run
```bash
git clone https://github.com/your-username/scroll-shot.git
cd scroll-shot
cargo build --release
```
The compiled standalone binary is placed at `target/release/scroll-shot` (only **~1.9MB**).

Run directly:
```bash
./target/release/scroll-shot
```

---

## 💻 Command-Line Interface (CLI)

```bash
# 1. Interactive selection mode: drag to select region
./scroll-shot

# 2. Single screenshot mode (skip scrolling capture)
./scroll-shot --single

# 3. Save directly to file (clipboard copy is always active)
./scroll-shot -o ./screenshot.png

# 4. Specific target rectangle (x, y, width, height)
./scroll-shot --rect 100,200,800,600

# 5. Background daemon mode for global hotkey (Ctrl+1)
./scroll-shot --daemon
```

### CLI Option Reference

| Option | Flag | Default | Description |
| :--- | :--- | :--- | :--- |
| `--output <PATH>` | `-o` | Auto-clipboard (optional file) | Output PNG image file path |
| `--rect <X,Y,W,H>` | `-r` | None (interactive) | Predefined capture rectangle |
| `--single` | | `false` | Single screenshot mode |
| `--daemon` | | `false` | Background daemon listening for Ctrl+1 |
| `--delay <MS>` | `-d` | `80` | Post-scroll settling debounce delay in ms |
| `--settle-timeout <MS>`| | `300` | Adaptive settling timeout for smooth-scrolling |
| `--step <COUNT>` | `-s` | `2` | Scroll tick count per injection |
| `--max-screens <N>`| `-m` | `20` | Maximum vertical screen height multiplier |
| `--preset <vm/native>`| | `native` | Tuning preset (`vm` optimizes cursor position) |
| `--verbose` | `-v` | `false` | Enable verbose alignment logging |

---

## ⌨️ Shortcut Reference

| Context | Shortcut | Action |
| :--- | :--- | :--- |
| **Global** | `Ctrl + 1` | Summon screen capture overlay |
| **Selection Mask** | Mouse Drag | Draw rectangular capture region |
| | `Enter` / Double-click | Capture selection and copy to clipboard |
| | `s` | Start scrolling capture mode |
| | `p` | Pin selection to desktop |
| | `e` | Enter annotation editor |
| | `o` | Extract text with OCR |
| | `ESC` | Cancel and dismiss |
| **Scrolling Recorder** | Mouse Wheel / Trackpad | Scroll page vertically |
| | `Enter` / Click `[Done]` | Finish scrolling capture and copy to clipboard |
| | `ESC` | Cancel scrolling capture |
| **Pin Window** | Left Mouse Drag | Move pin window position |
| | Mouse Wheel | Scale image (0.15x ~ 4.0x) |
| | `0` | Reset image to 1:1 original scale |
| | `Ctrl` / `Shift` + Wheel | Adjust opacity (30% ~ 100%) |
| | Double-click / `ESC` | Close pin window |
| **OCR Dialog** | Left Mouse Drag | Highlight select text segment |
| | `Ctrl + C` / `c` | Copy highlighted text (or all text if none selected) |
| | `Ctrl + A` | Select all text |
| | `ESC` | Close OCR dialog |

---

## ❓ FAQ & Troubleshooting

### Q1: Does Chinese or foreign text render as missing boxes or garbled symbols?
- **No**. `scroll-shot` includes a pure-Rust dynamic FreeType rasterization engine that loads native Linux CJK vector fonts (e.g., Noto Sans CJK, Droid Sans) with antialiased rendering, natural baseline positioning, and full-width/half-width alignment.

### Q2: Why does the OCR window prompt that the engine is not installed?
- `scroll-shot` adheres to a lightweight (~1.9MB) and privacy-first design, avoiding bundled multi-megabyte models. It interfaces cleanly with system Tesseract. On Ubuntu/Debian, install with `sudo apt install -y tesseract-ocr tesseract-ocr-chi-sim`.

### Q3: Does the floating pin window stutter during drag or resize?
- **No**. The window pipeline utilizes event coalescing and immediate graphic context flushes (`conn.flush()`) to ensure smooth 60~144Hz performance on 4K HiDPI screens.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
