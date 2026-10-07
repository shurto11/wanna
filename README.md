# wanna

やりたいことを「縦軸 × clau度」の4区分に置いて、自分の好きな順に並べておくリスト。
ターミナルで使う TUI (`wanna`) が本体で、自宅サーバーの `wannad` を通してブラウザ版と同期する。

## タブ

| タブ | 中身 |
|---|---|
| **Want** | やりたいこと。縦軸はエネルギー（取りかかるのに要る気力）。日時は持たない |
| **Must** | 次にやること。縦軸は重要度。日時を持てて、日時を入れると区分内で日時順に並ぶ |
| **Done** | やったもの。Want・Must を混ぜて、やった日の新しい順 |
| **Archive** | やってはいないが、リストに置くほどでもなくなったもの |
| **Memo** | 区分を持たないメモ。日付と名前だけ並び、本文は vim で書く |

横軸の **clau度** は Claude Code に任せられる度合い。右が高い。

## インストール

Rust (cargo) が要る。

```sh
make install-tui          # ~/.cargo/bin/wanna に入る。更新も同じコマンド
wanna                     # 起動
```

### サーバーと同期する

`~/.config/wanna/config.toml` に wannad の場所と token を書く（例: `tui.config.example.toml`）。

```toml
server = "http://100.x.x.x:8787"
token = "change-me"
```

環境変数 `WANNA_SERVER` / `WANNA_TOKEN` があればそちらが優先。
設定が無ければローカルのみで動き、変更は手元に溜めておいて、設定したときにまとめて送る。
オフラインでも使え、つながったら送り直す。

## キー操作

どの画面でも `[` / `]`（または `Shift+Tab` / `Tab`）でタブを切り替え、`q` で終了する。

### Want / Must

| キー | 動作 |
|---|---|
| `h` `j` `k` `l` / 矢印 | 区分と行の移動。`j` `k` は区分の端で上下の区分へ抜ける |
| `1`〜`9` `0` | 区分内の 1〜10 番目へ |
| `g` / `G` | 区分の先頭 / 末尾へ |
| `n` | カーソルのある区分に追加 |
| `e` / `Enter` | 編集（名前・縦軸・clau度・日時） |
| `s` | 日時を編集（Must のみ） |
| `m` | メモをエディタで開く |
| `J` / `K` | 区分内で下 / 上へ並べ替え |
| `X` | もう一方のリストへ移す（同じ区分の末尾に入る） |
| `t` | やった（Done へ） |
| `a` | しまう（Archive へ） |
| `d` | 削除（確認あり） |
| `r` | 今すぐ同期 |

### 編集ポップアップ

| キー | 動作 |
|---|---|
| `↑` `↓`（高低の欄では `j` `k` も） | 欄の移動 |
| `h` / `l` / `Space` | 高 / 低 / 反転 |
| `Tab` | メモをエディタで編集 |
| `Enter` / `Ctrl+S` | 保存 |
| `Esc` | やめる |

日時は `2026-09-25` / `09-25 14:00` / `明日 18:00` / `金` のように書ける。空にすると消える。

### Done / Archive

| キー | 動作 |
|---|---|
| `j` `k` `g` `G` | 移動 |
| `m` | メモをエディタで開く |
| `u` | 元のリストに戻す |
| `t` | やった（Archive のみ） |
| `d` | 削除 |
| `Esc` | 直前のリストへ戻る |

### Memo

| キー | 動作 |
|---|---|
| `j` `k` `g` `G` | 移動 |
| `n` | 名前を入れて作り、そのまま本文をエディタで開く |
| `Enter` / `m` | 本文をエディタで開く |
| `e` | 名前を変える |
| `d` | 削除 |

### エディタ

メモは `$VISUAL` → `$EDITOR` → `vim` の順に見つかったもので開く。
Want / Must の空のメモには見出し（`Why` / `Done when` / `Next steps` / `Log` / `Open questions`）を入れて開き、
見出しのまま閉じたら空に戻す。Memo には何も入れない。

## コマンドライン

引数を付けると TUI を開かずに1件を読み書きする（Claude Code の `/wanna` スキルが使う）。
常にサーバーへ直接読み書きする。

```
wanna ls [--want|--must|--memo] [--done|--archived] [--json]
wanna show [ID] [--json]
wanna new <タイトル> [--want] [--lo] [--no-clau] [--link]
wanna log [-c] <ID> <テキスト...>
wanna note <ID> <見出し> [テキスト...|-]
wanna done <ID>
wanna archive <ID>
wanna link [ID] / wanna unlink
```

ID は全体か末尾の一部（`ls` の左端）。`@` はこのディレクトリに紐付けた1件
（リポジトリ直下の `.claude/wanna.local.toml`）。

## 構成

| ディレクトリ | 中身 |
|---|---|
| `tui/` | TUI とコマンドライン (`wanna`)。手元のキャッシュは `~/.config/wanna/cache.db` |
| `server/` | バックエンド (`wannad`)。SQLite に保存し、SSE で変更を知らせる。ブラウザ版を埋め込んで配る |
| `web/` | ブラウザ版 (TypeScript + Vite, PWA)。Memo は CodeMirror 6 の vim モードで書ける |
| `core/` | 共有部分（モデル・日時・並び順・API クライアント） |
| `migrate/` | Google Tasks からの一度きりの移行 |
| `deploy/` | systemd のユニット、デプロイとバックアップのスクリプト |

```sh
make test                 # Rust と web のテスト
make deploy               # wannad を入れ替える（DB をバックアップし、応答が無ければ元に戻す）
make deploy DEPLOY_HOST=… # 入れ替え先を変える（既定は dynabook）
```

サーバーを初めて置くときは、サーバー側で `make install-server` を実行し、
`~/wanna/config.toml`（例: `config.example.toml`）に bind・token・db を書いて
`deploy/wannad.service` を有効にする。到達性は Tailscale を前提にしていて、外には開けない。
