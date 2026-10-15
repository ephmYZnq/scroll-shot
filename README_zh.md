# scroll-shot

<div align="center">

**专为 Linux 桌面打造的极轻量、高分辨率、像素级无损滚动截屏与贴图工具。**

[![GitHub Release](https://img.shields.io/github/v/release/ephmYZnq/scroll-shot?color=3388ff&logo=github)](https://github.com/ephmYZnq/scroll-shot/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Binary Size](https://img.shields.io/badge/Binary%20Size-~1.9MB-blue.svg)](https://github.com/ephmYZnq/scroll-shot/releases)

[English](README.md) | [简体中文](README_zh.md) | [🚀 直接下载 v0.3.1 发行版](https://github.com/ephmYZnq/scroll-shot/releases/latest)

</div>

---

`scroll-shot` 是一个采用纯 Rust 编写的系统级截图与长截图工具，专注于在 Linux 桌面环境（**同时支持 X11 与 GNOME Wayland**）下提供**极速冷启动、低内存占用、单二进制绿色免安装（<2MB）、零重型 GUI 依赖**以及 **100% 点对点原画质长图拼接**能力。

---

## 🌟 核心特性概览

### 1. 📋 直达系统剪贴板（即截即贴，无需存盘）
- 截完图（无论是普通截图、批注截图还是长截图）**自动写入 X11 剪贴板**，无需生成多余文件落盘。
- 切换到聊天软件（微信/Slack/Telegram）、邮件或文档直接 **`Ctrl+V`** 即可粘贴！
- 如有存档需要，亦可通过 `-o <PATH>` 显式另存为文件。

### 2. 📌 贴图置顶桌面（Pin to Desktop）
- 类似 Snipaste / PixPin 的桌面悬浮贴图能力：
  - **任意拖拽**：鼠标按住窗口任意位置即可移动，采用 X11 鼠标轨迹压缩合并技术，即便在 4K 高分屏下拖拽依然丝滑顺畅。
  - **无损插值缩放**：滚轮支持 0.15x ~ 4.0x 双线性/最近邻无损缩放，按键盘 `0` 瞬间恢复 1:1 原始尺寸。
  - **透明度实时调节**：按住 `Ctrl` 或 `Shift` + 滚轮实时微调窗口透明度（Alpha 30% ~ 100%）。
  - **便捷关闭**：双击窗口、按 `ESC` 或点击右上角关闭按钮退出置顶。

### 3. 🎨 专业级截图批注（Advanced Annotator）
- **步骤序号气泡 ①②③**（快捷键 `n`）：点击自动放置递增序号气泡，清晰标示操作指引流程，支持 `Ctrl+Z` 撤销自动回退计数。
- **交互式打字文本标注 T Text**（快捷键 `t`）：直接在截图中打字输入文字，支持光标回退修改与回车提交，文字自带 1px 轮廓阴影抗干扰。
- **经典工具全覆盖**：矩形框、箭头、马赛克、高亮划线笔、4 色调色板。

### 4. 🔤 离线多语言 OCR 与交互式文本提取（Offline OCR）
- **秒级本地离线提取**：选区工具条点击 **`OCR`**（或按 `o`），即可秒级提取选中区域中的代码、报错或文本。
- **多语言混合识别**：自动探测并协同调度系统安装的中文（`chi_sim`）、繁体（`chi_tra`）、英文（`eng`）及其他语言模型，实现混合排版无缝提取。
- **FreeType 矢量字形原生渲染**：动态接入系统级 FreeType 引擎，高清抗锯齿呈现中英文字形，彻底告别字符乱码。
- **鼠标划词高亮选择与精确复制**：
  - 支持鼠标在文本框中自由拖选任意单行或跨多行文本，实时渲染皇家蓝高亮色块。
  - **局部复制联动**：有高亮选区时，按 `Ctrl+C`、单键 `C` 或点击复制按钮**仅复制选中的文本片段**；未框选时自动复制全文。
  - 支持 `Ctrl+A` 一键全选。
  - 提取文字自动写入 X11 `CLIPBOARD`（`Ctrl+V`）与 `PRIMARY`（鼠标中键）剪贴板。

### 5. 🎯 原生像素图标与精致 UI
- 工具栏全面采用原生 8x8 定制点阵矢量图标（相机、图钉、铅笔、OCR 扫描、滚动箭头、关闭），不依赖系统 Emoji 字体，杜绝任何缺失方块（`☒`），在任何 Linux 发行版与桌面环境下均呈现一致的锐利视觉。

### 6. 📜 自由手动滚动长截图（Human-Driven Live Stitching）
- 告别机械死板的自动滚屏！进入长截图后，屏幕上方显示精致的高度录制条，右侧实时显示**等比例拼接长图微缩预览**。
- 用户使用**鼠标滚轮、触控板或直接拖拽页面滚动条**自由滚动，程序以高帧率实时捕捉新增内容并无损拼接。
- 滚到满意位置后，点击录制条上的 **`[Done]`** 按钮（或按回车），一键完成并写入剪贴板。

### 7. 🖥️ X11 & GNOME Wayland 双引擎自适应抓屏 + 全局快捷键
- **双引擎自适应抓屏**：在 X11 (`Xorg`) 下采用原生 `XGetImage` 零拷贝抓屏；在 GNOME Wayland 下自动无缝切换至合成器底层 D-Bus 抓屏（`libgio-2.0.so.0` + `/dev/shm` 纯内存中转），无闪屏、无权限弹窗。
- 启动后自动弹出对标系统原生体验的全屏暗色冻结遮罩，支持鼠标拖拽框选、八方向手柄缩放调节。
- **全局快捷键【Ctrl + 1】秒启**：专为全屏开发、虚拟机操作定制，无需切换窗口找图标。

---

## 📐 处理管线流程 (Pipeline)

```
┌──────────────────┐     ┌───────────────────┐     ┌───────────────────────┐
│ 1. 选区交互      │ ──> │ 2. 抓帧与滚动循环 │ ──> │ 3. 1D 像素级无损缝合  │
│ 鼠标拖拽框选 /    │     │ XShm 抓取当前帧   │     │ 提取上一帧底部模板    │
│ CLI 指定区域     │     │ 用户自由滚动页面  │     │ 寻找精确垂直位移 Δy   │
└──────────────────┘     └───────────────────┘     └──────────┬────────────┘
                                                              │
                                                              ▼
                                                   ┌───────────────────────┐
                                                   │ 4. 实时微缩预览与输出 │
                                                   │ 动态画布无损扩容      │
                                                   │ 一键直达系统剪贴板    │
                                                   └───────────────────────┘
```

---

## 🚀 快速开始

### 1. 系统要求与环境准备
- **操作系统**：Linux（首发支持 X11 架构，GNOME / KDE / XFCE 等桌面环境）。
- **编译工具**：Rust 1.75+ (Cargo)。
- **可选依赖：本地离线 OCR 组件**：
  - `scroll-shot` 坚持纯离线、隐私安全与小体积原则，通过管道对接轻量的 `tesseract-ocr` 引擎：
    - **Ubuntu / Debian / Linux Mint / Deepin**：
      ```bash
      sudo apt install -y tesseract-ocr tesseract-ocr-chi-sim
      ```
    - **Arch Linux / Manjaro**：
      ```bash
      sudo pacman -S tesseract tesseract-data-chi_sim tesseract-data-eng
      ```
    - **Fedora / RHEL**：
      ```bash
      sudo dnf install -y tesseract tesseract-langpack-chi_sim tesseract-langpack-eng
      ```
  - *注：若系统未安装，点击 OCR 时弹窗亦会友好提示，并提供一键复制安装命令按钮 `[Copy Command]`。*

### 2. 构建与运行
```bash
git clone https://github.com/your-username/scroll-shot.git
cd scroll-shot
cargo build --release
```
编译产物位于 `target/release/scroll-shot`（单文件二进制体积仅约 **1.9MB**）。

直接运行：
```bash
./target/release/scroll-shot
```

---

## 💻 命令行用法 (CLI)

```bash
# 1. 默认交互模式：启动后鼠标拖拽框选区域
./scroll-shot

# 2. 单张普通截图模式（跳过长截图流程，选完即复制）
./scroll-shot --single

# 3. 指定输出文件路径（默认已自动复制到剪贴板）
./scroll-shot -o ./screenshot.png

# 4. 指定固定坐标区域 (x, y, 宽度, 高度) 跳过鼠标框选交互
./scroll-shot --rect 100,200,800,600

# 5. 常驻后台全局快捷键监听模式（按下 Ctrl+1 即可唤起）
./scroll-shot --daemon
```

### CLI 参数列表

| 参数项 | 简写 | 默认值 | 说明 |
| :--- | :--- | :--- | :--- |
| `--output <PATH>` | `-o` | 自动剪贴板 (可选文件) | 另存为 PNG 文件路径 |
| `--rect <X,Y,W,H>` | `-r` | 无 (启动交互框选) | 截取矩形范围 |
| `--single` | | `false` | 单张普通截图模式 |
| `--daemon` | | `false` | 常驻后台监听全局快捷键（Ctrl+1） |
| `--delay <MS>` | `-d` | `80` | 滚轮后基础防抖等待延迟 (毫秒) |
| `--settle-timeout <MS>`| | `300` | 画面静止检测超时(毫秒)，自适应等待平滑滚动停稳 |
| `--step <COUNT>` | `-s` | `2` | 每次滚动发送的滚轮刻度次数 |
| `--max-screens <N>`| `-m` | `20` | 最大允许拼接的屏幕倍数，防死循环保护 |
| `--preset <vm/native>`| | `native` | 预设模式（`vm` 模式优化光标位置与等待停稳） |
| `--verbose` | `-v` | `false` | 输出详细的匹配对齐日志与位移数值 |

---

## ⌨️ 快捷键速查表

| 场景 | 快捷键 | 功能说明 |
| :--- | :--- | :--- |
| **全局** | `Ctrl + 1` | 随时呼出截图选区遮罩 |
| **遮罩选区** | 鼠标拖拽 | 自由拉取截图区域 |
| | `Enter` / 双击选区 | 快速完成普通截图并复制进剪贴板 |
| | `s` | 启动长截图模式 |
| | `p` | 贴图置顶到桌面 |
| | `e` | 进入截图批注编辑模式 |
| | `o` | 提取选中区域文字 (OCR) |
| | `ESC` | 取消并退出 |
| **长截图录制** | 鼠标滚轮 / 触控板 | 自由向下滚动页面 |
| | `Enter` / 点击 `[Done]` | 完成长图拼接并复制进剪贴板 |
| | `ESC` | 取消长截图 |
| **贴图置顶 (Pin)** | 鼠标左键拖拽 | 移动贴图窗口位置 |
| | 鼠标滚轮 | 缩放贴图尺寸 (0.15x ~ 4.0x) |
| | `0` | 重置贴图为 1:1 原始尺寸 |
| | `Ctrl` / `Shift` + 滚轮 | 调节贴图窗口透明度 (30% ~ 100%) |
| | 双击 / `ESC` | 关闭当前贴图 |
| **OCR 文本预览** | 鼠标左键拖拽 | 划词高亮选择任意文本 |
| | `Ctrl + C` / `c` | 复制选中文字（无选区则复制全文） |
| | `Ctrl + A` | 全选文本框内容 |
| | `ESC` | 关闭 OCR 窗口 |

---

## ❓ 常见问题 (FAQ)

### Q1: 识别中文会出现方框或者乱码吗？
- **不会**。`scroll-shot` 内置了纯 Rust 动态 FreeType 光栅化引擎，自动加载 Linux 系统自带的矢量中文开源字体（如 Noto Sans CJK、Droid Sans 等），笔画清晰平滑，且自动适配全角与半角字符间距。

### Q2: 为什么点击 OCR 提示未检测到引擎？
- `scroll-shot` 坚持绿色极简体积（~1.9MB）与本地隐私安全原则，未捆绑数十兆的庞大预训练模型，而是通过系统管道对接开源成熟的 Tesseract 引擎。
- 在 Ubuntu/Debian 系统只需执行一行命令：`sudo apt install -y tesseract-ocr tesseract-ocr-chi-sim` 即可秒级启用中英文混合识别。

### Q3: 贴图置顶在移动或缩放时会卡顿吗？
- **不会**。底层管线针对 X11 进行了高帧率优化，启用鼠标运动压缩合并与即时显存通道冲刷，全屏及 4K 环境下均可保持 60~144Hz 的跟手流畅度。

---

## 📄 开源许可证

本项目基于 [MIT License](LICENSE) 开源发布。
