# Kitty Graphics Protocol — 実装状況

bcon の [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) 実装状況を他のターミナルと比較します。

最終更新: 2026-03-31 (v1.3.0)

## 転送モード

| モード | キー | bcon | kitty | Ghostty | WezTerm |
|--------|------|------|-------|---------|---------|
| Direct (base64インライン) | `t=d` | Yes | Yes | Yes | Yes |
| ファイルパス | `t=f` | Yes | Yes | Yes | Yes |
| 一時ファイル | `t=t` | Yes | Yes | Yes | Yes |
| 共有メモリ | `t=s` | Yes | Yes | Yes | Yes |

4モードすべてデフォルトで有効。`[security] allow_kitty_remote = false` で無効化可能。

## アクション

| アクション | キー | bcon | kitty | Ghostty | WezTerm |
|-----------|------|------|-------|---------|---------|
| 送信のみ | `a=t` | Yes | Yes | Yes | Yes |
| 送信+表示 | `a=T` | Yes | Yes | Yes | Yes |
| 表示 | `a=p` | Yes | Yes | Yes | Yes |
| クエリ | `a=q` | Yes | Yes | Yes | Yes |
| 削除 | `a=d` | Yes | Yes | Yes | 部分的 |
| フレームデータ | `a=f` | Yes | Yes | No | Yes |
| アニメーション制御 | `a=a` | Yes | Yes | No | 部分的 |
| フレーム合成 | `a=c` | Yes | Yes | No | Yes |

## 削除ターゲット (`a=d`)

| 対象 | キー | bcon | kitty | Ghostty | WezTerm |
|------|------|------|-------|---------|---------|
| 全画像 | `d=a/A` | Yes | Yes | Yes | Yes |
| ID指定 | `d=i/I` | Yes | Yes | Yes | Yes |
| 番号指定 | `d=n/N` | Yes | Yes | Yes | No |
| カーソル位置 | `d=c/C` | Yes | Yes | Yes | No |
| セル座標 | `d=p/P` | Yes | Yes | Yes | No |
| 列指定 | `d=x/X` | Yes | Yes | Yes | No |
| 行指定 | `d=y/Y` | Yes | Yes | Yes | No |
| Z-index指定 | `d=z/Z` | Yes | Yes | Yes | No |
| ID範囲 | `d=r/R` | Yes | Yes | Yes | No |
| アニメーションフレーム | `d=f/F` | Yes | Yes | Yes (no-op) | No |

## 画像管理

| 機能 | bcon | kitty | Ghostty | WezTerm |
|------|------|-------|---------|---------|
| ストレージ方式 | テクスチャキャッシュ (HashMap) | アウトオブバンド (verstable hashmap) | アウトオブバンド (Pin追跡) | セル付加方式 |
| スクロール追跡 | Yes (絶対行座標) | Yes (行オフセット) | Yes (Pin) | Yes (セル内蔵) |
| Z-order | Yes (2パス描画) | Yes (3パス) | Yes (3レイヤー) | Yes (z-indexソート) |
| Unicode placeholder | Yes (U+10EEEE) | Yes | Yes | No |
| ストレージ上限 | 128テクスチャ | 320MB | 320MB | 320MB |
| 画面クリア (`ESC[2J`) で画像削除 | Yes | Yes | Yes | Yes |

## 画像フォーマット

| フォーマット | キー | bcon | kitty | Ghostty | WezTerm |
|-------------|------|------|-------|---------|---------|
| RGBA (32bpp) | `f=32` | Yes | Yes | Yes | Yes |
| RGB (24bpp) | `f=24` | Yes | Yes | Yes | Yes |
| PNG | `f=100` | Yes | Yes | Yes | Yes |

## その他の機能

| 機能 | bcon | kitty | Ghostty | WezTerm |
|------|------|-------|---------|---------|
| チャンク転送 (`m=1`) | Yes | Yes | Yes | Yes |
| zlib圧縮 (`o=z`) | Yes | Yes | Yes | Yes |
| レスポンス (`q=0/1/2`) | Yes | Yes | Yes | Yes |
| カーソル移動 (`C=0/1`) | Yes | Yes | Yes | Yes |
| セル内オフセット (`X`, `Y`) | Yes | Yes | Yes | ? |
| 表示サイズ (`c`, `r`) | Yes | Yes | Yes | ? |
| 相対配置 (`P`, `Q`) | Yes | Yes | Yes | No |

## 既知の制限

- Z<0 画像: 画像領域上のテキスト背景が黒くなる（LCDサブピクセルレンダリングの制約、kitty でも同様）

## テストスイート

bcon 上でテストを実行:

```bash
python3 tests/generate-test-images.py
bash tests/kitty-graphics-test.sh
```

26テスト: 表示、削除（全ターゲット）、スクロール追跡、オーバーレイ、Z-order、セルオフセット、相対配置、ID範囲削除、クリック時の永続性。
