# Keybinds

English | **[Japanese](keybinds.ja.md)**

## Default Keybinds

| Action | Default | Vim | Emacs | Description |
|--------|---------|-----|-------|-------------|
| Copy | `Ctrl+Shift+C` | same | `Ctrl+Shift+W` | Copy selection to clipboard |
| Paste | `Ctrl+Shift+V` | same | `Ctrl+Shift+Y` | Paste from clipboard |
| Screenshot | `PrintScreen` | same | same | Save screenshot as PNG |
| Search | `Ctrl+Shift+F` | same | `Ctrl+Shift+S` | Search in scrollback |
| Copy Mode | `Ctrl+Shift+Space` | same | `Ctrl+Shift+M` | Enter vim-like copy mode |
| Font + | `Ctrl+Plus` | same | same | Increase font size |
| Font - | `Ctrl+Minus` | same | same | Decrease font size |
| Font Reset | `Ctrl+0` | same | same | Reset font size |
| Scroll Up | `Shift+PageUp` | `Ctrl+Shift+U` | `Alt+Shift+V` | Scroll back |
| Scroll Down | `Shift+PageDown` | `Ctrl+Shift+D` | `Alt+Shift+N` | Scroll forward |
| Notifications | `Ctrl+Shift+N` | same | same | Toggle notification panel |
| Mute | `Ctrl+Shift+M` | same | `Alt+Shift+M` | Toggle notification mute |
| Split Right | `Ctrl+Shift+Enter` | same | same | Split pane horizontally |
| Split Down | `Ctrl+Shift+D` | `Ctrl+Shift+\` | `Ctrl+Shift+D` | Split pane vertically |
| Close Pane | `Ctrl+Shift+W` | same | `Alt+Shift+W` | Close active pane |
| Pane Navigate | `Ctrl+Shift+Arrow` | `Ctrl+Shift+H/J/K/L` | `Ctrl+Shift+Arrow` | Move focus between panes |
| Pane Resize | `Ctrl+Shift+Alt+Arrow` | `Ctrl+Shift+Alt+H/J/K/L` | `Ctrl+Shift+Alt+Arrow` | Resize active pane |
| Zoom Pane | `Ctrl+Shift+Z` | same | same | Toggle pane zoom |
| New Tab | `Ctrl+Shift+T` | same | same | Open new tab |
| Close Tab | `Ctrl+Shift+Q` | same | same | Close active tab |
| Next Tab | `Ctrl+Shift+PageDown` | same | same | Switch to next tab |
| Prev Tab | `Ctrl+Shift+PageUp` | same | same | Switch to previous tab |

## Custom Keybinds

Multiple keys can be assigned to a single action in config:

```toml
[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
```

## Copy Mode Keys (Vim-like)

Enter copy mode with `Ctrl+Shift+Space` (default) to navigate and select text using keyboard.

| Key | Action |
|-----|--------|
| `h/j/k/l` | Move cursor |
| `w/b` | Word forward/backward |
| `0/$` | Line start/end |
| `g/G` | Top/bottom of buffer |
| `v` | Start/toggle selection |
| `y` | Yank (copy) and exit |
| `/` | Search |
| `Esc` | Exit copy mode |
