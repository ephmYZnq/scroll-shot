#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/scroll-shot"

echo "🔧 配置 GNOME 全局快捷键: Ctrl + 1 -> $BIN"

gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/scroll-shot/ name "Scroll Shot"
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/scroll-shot/ command "$BIN"
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/scroll-shot/ binding "<Ctrl>1"

CURRENT=$(gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings)
PATH_ENTRY="'/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/scroll-shot/'"

if [[ "$CURRENT" != *"$PATH_ENTRY"* ]]; then
    if [[ "$CURRENT" == "@as []" || "$CURRENT" == "[]" ]]; then
        gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "[$PATH_ENTRY]"
    else
        NEW_LIST="${CURRENT%]*}, $PATH_ENTRY]"
        gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "$NEW_LIST"
    fi
fi

echo "✅ 配置完成！随时按下【Ctrl + 1】即可直接呼出 scroll-shot 截图！"
