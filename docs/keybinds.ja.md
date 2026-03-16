# キーバインド

**[English](keybinds.md)** | Japanese

## デフォルトキーバインド

| アクション | デフォルト | Vim | Emacs | 説明 |
|-----------|-----------|-----|-------|------|
| コピー | `Ctrl+Shift+C` | 同左 | `Ctrl+Shift+W` | 選択をクリップボードにコピー |
| ペースト | `Ctrl+Shift+V` | 同左 | `Ctrl+Shift+Y` | クリップボードからペースト |
| スクリーンショット | `PrintScreen` | 同左 | 同左 | PNG でスクリーンショット保存 |
| 検索 | `Ctrl+Shift+F` | 同左 | `Ctrl+Shift+S` | スクロールバック内検索 |
| コピーモード | `Ctrl+Shift+Space` | 同左 | `Ctrl+Shift+M` | Vim ライクコピーモード開始 |
| フォント拡大 | `Ctrl+Plus` | 同左 | 同左 | フォントサイズ拡大 |
| フォント縮小 | `Ctrl+Minus` | 同左 | 同左 | フォントサイズ縮小 |
| フォントリセット | `Ctrl+0` | 同左 | 同左 | フォントサイズリセット |
| スクロールアップ | `Shift+PageUp` | `Ctrl+Shift+U` | `Alt+Shift+V` | 上にスクロール |
| スクロールダウン | `Shift+PageDown` | `Ctrl+Shift+D` | `Alt+Shift+N` | 下にスクロール |
| 通知パネル | `Ctrl+Shift+N` | 同左 | 同左 | 通知パネルの開閉 |
| 通知ミュート | `Ctrl+Shift+M` | 同左 | `Alt+Shift+M` | トースト通知のミュート切替 |
| 右に分割 | `Ctrl+Shift+Enter` | 同左 | 同左 | ペインを水平分割 |
| 下に分割 | `Ctrl+Shift+D` | `Ctrl+Shift+\` | `Ctrl+Shift+D` | ペインを垂直分割 |
| ペイン閉じる | `Ctrl+Shift+W` | 同左 | `Ctrl+Shift+X` | アクティブペインを閉じる |
| ペイン移動 | `Ctrl+Shift+Arrow` | `Ctrl+Shift+H/J/K/L` | `Ctrl+Shift+Arrow` | ペイン間のフォーカス移動 |
| ペインリサイズ | `Ctrl+Shift+Alt+Arrow` | `Ctrl+Shift+Alt+H/J/K/L` | `Ctrl+Shift+Alt+Arrow` | アクティブペインのリサイズ |
| ペインズーム | `Ctrl+Shift+Z` | 同左 | 同左 | ペインズーム切替 |
| 新規タブ | `Ctrl+Shift+T` | 同左 | 同左 | 新しいタブを開く |
| タブ閉じる | `Ctrl+Shift+Q` | 同左 | 同左 | アクティブタブを閉じる |
| 次のタブ | `Ctrl+Shift+PageDown` | 同左 | 同左 | 次のタブに切替 |
| 前のタブ | `Ctrl+Shift+PageUp` | 同左 | 同左 | 前のタブに切替 |

## カスタムキーバインド

1つのアクションに複数のキーを割り当て可能:

```toml
[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
```

## コピーモードキー (Vim ライク)

`Ctrl+Shift+Space` (デフォルト) でコピーモードに入り、キーボードでテキストをナビゲーション・選択できます。

| キー | アクション |
|-----|----------|
| `h/j/k/l` | カーソル移動 |
| `w/b` | 単語前方/後方 |
| `0/$` | 行頭/行末 |
| `g/G` | バッファ先頭/末尾 |
| `v` | 選択開始/切り替え |
| `y` | ヤンク (コピー) して終了 |
| `/` | 検索 |
| `Esc` | コピーモード終了 |
