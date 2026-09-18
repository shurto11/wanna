# wanna 設計

エネルギー × clau度 の4区分で「やりたいこと」を並べておくリスト。
TUI / ブラウザ / Android の3クライアントが1つのバックエンドを共有する。

これはタスク管理ではない。期限も優先順位も無く、**やりたいことを4つの区分に置いて、
自分の好きな順に並べておく**もの。やったものは「やったこと」として見返せる。

同期方式の調査と他案との比較は `calendar-tui/sync-design.md` を参照。

## 決定事項 (2026-09-18)

| 項目 | 決定 |
|------|------|
| 名前 | **wanna**。TUI が `wanna`、サーバーが `wannad` |
| データの正 | **自前バックエンド (SQLite)**。Google Tasks には依存しない |
| Google Tasks | 一度だけ移行して切り離す。以後 API は呼ばない |
| ホスト | 自宅の Ubuntu Server ノートPC で常時稼働 |
| 到達性 | **Tailscale**。tailnet 内のみ公開、ポート開放なし |
| クライアント | TUI (Rust/ratatui) / ブラウザ (TypeScript + Vite, PWA) / Android (Kotlin, 後回し) |
| ウィジェット | **初期は実装しない**。ただし push (SSE) は最初から用意して後で乗せられるようにする |
| calendar-tui | カレンダー専用に戻す。タスク画面 (`matrix.rs` 等) は本リポジトリへ移す |

## 用語

| 用語 | コード上 | 意味 |
|---|---|---|
| やりたいこと | `Want` / `wants` | このアプリが扱う唯一のもの |
| やった | `done_at` | 達成した日時。入っていれば「やったこと」側に回る |
| 区分 | `(energy, clau)` | エネルギー高低 × clau度高低 の4つ |
| 並び | `pos` | 区分内での順番。ユーザーが決める |

「タスク」「完了」「優先順位」という語は使わない。

## calendar-tui / todo-design.md からの変更点

| 旧 | 新 |
|---|---|
| タスク管理 | **やりたいことリスト** |
| 重要度 0〜10 | **エネルギー 高/低** の2段階 |
| clau度 0〜10 | **clau度 高/低** の2段階 |
| 連続値のマトリックスに点をプロット | **4区分それぞれをリストとして表示** (2×2 グリッド) |
| 優先順位 `(11-imp)*(11-clau)*日数` で自動算出 | **削除**。順番はユーザーが決める（デフォルトは追加順） |
| 日付・時刻 (due) | **削除** |
| タスクリスト（カテゴリー） | **削除**。区分が唯一の分類軸になる |
| スタック（行き詰まり + カテゴリー + 詳細） | **削除** |
| 完了したら消えるだけ | **「やったこと」として達成日順に見返せる** |

結果として、1件が持つのは **名前 / メモ / エネルギー / clau度 / 区分内の並び / やった日時** だけになる。
`priority.rs` は不要になり、`meta.rs` の `StackCategory` も消える。

---

## 1. 構成

```
wanna/
├── Cargo.toml          workspace
├── core/               モデル / 並び順キー / API クライアント (tui と server が共有)
├── server/             axum + rusqlite。API と静的ファイル配信 → wannad
├── tui/                ratatui クライアント           → wanna
├── web/                TypeScript + Vite の PWA
├── android/            Kotlin + Compose (Phase 5)
└── migrate/            Google Tasks → SQLite の一度きりの移行ツール
```

`core` を server と tui で共有する。web は TypeScript 側に並び順キーの生成だけ再実装する。

### 依存の方針

- SQLite は **rusqlite の `bundled` feature** を使う。libsqlite3 を apt で入れる必要がなくなる。
- server は単一バイナリ。web のビルド成果物は `include_dir` で埋め込む。
- 設定とキャッシュは `~/.config/wanna/`。

---

## 2. データモデル

### 区分 (quadrant)

エネルギーと clau度がそれぞれ2値なので、区分は4つ。DB には2つの真偽値として持ち、
「区分」という単一カラムは作らない（軸ごとにトグルできるほうが素直なため）。

```
                 clau低              clau高
エネルギー高   自分でやる (HL)    claudeとやる (HH)
エネルギー低   片手間 (LL)        流し込む (LH)
```

区分名は表示用のラベルなので、実装では `(energy, clau)` の組で扱う。ラベルは後から変えられる。

### スキーマ

```sql
-- 同期のための単調増加リビジョン。書き込みのたびに +1 する。
CREATE TABLE rev_counter (
  id  INTEGER PRIMARY KEY CHECK (id = 0),
  rev INTEGER NOT NULL
);

CREATE TABLE wants (
  id         TEXT    PRIMARY KEY,           -- UUIDv7 (クライアント生成)
  title      TEXT    NOT NULL,
  notes      TEXT    NOT NULL DEFAULT '',
  energy     INTEGER NOT NULL CHECK (energy IN (0, 1)),  -- 1 = 高
  clau       INTEGER NOT NULL CHECK (clau   IN (0, 1)),  -- 1 = 高
  pos        TEXT    NOT NULL,              -- 区分内の並び順キー (後述)
  done_at    TEXT,                          -- RFC3339 / NULL ならまだやっていない
  deleted    INTEGER NOT NULL DEFAULT 0,
  rev        INTEGER NOT NULL,
  created_at TEXT    NOT NULL               -- RFC3339
);

CREATE INDEX idx_wants_rev      ON wants(rev);
CREATE INDEX idx_wants_quadrant ON wants(energy, clau, pos);
CREATE INDEX idx_wants_done     ON wants(done_at);
```

設計上の要点:

- **ID はクライアントが UUIDv7 で生成する。** オフラインで追加でき、再送しても重複しない（冪等 upsert）。
- **削除は tombstone** (`deleted = 1`)。物理削除すると「削除済み」と「まだ同期していない新規」を区別できない。
  `done_at` が入ったものは削除ではない。消したいときだけ `deleted` を立てる。
- **`rev` はサーバーが採番する整数**。タイムスタンプを使うと端末間の時計ずれで順序が壊れる。
  `rev` なら全書き込みに全順序が付き、差分取得が `WHERE rev > ?` だけで済む。
- テーブルは `wants` 1枚だけ。リストもスタックも無くなったので、他のテーブルは要らない。

### 並び順 — fractional index

「デフォルトは追加順、ユーザーが自由に変更」を素直に満たすため、`pos` は
**整数の連番ではなく、辞書順に比較できる文字列キー**にする（Figma などが使う fractional indexing）。

```
A と B の間に挿入したい  →  pos = midpoint(A.pos, B.pos)
先頭に挿入              →  pos = midpoint(None, 先頭.pos)
末尾に追加（新規追加）   →  pos = midpoint(末尾.pos, None)
```

キーは base62 (`0-9A-Za-z`) の文字列。中間値が取れなくなったら末尾に1文字足すので、
何回でも分割できる。

これを選ぶ理由は、**並べ替えが1件の更新だけで完結する**こと。
整数の連番だと1つ動かすたびに後続の全件を書き換える必要があり、
オフライン編集と複数クライアントの同期では衝突を量産する。fractional index なら
`PATCH /api/wants/:id {"pos": "..."}` が1本飛ぶだけで済む。

- `pos` は区分ごとに独立したキー空間。区分をまたいで移動したら、移動先の末尾の `pos` を振り直す。
- 2端末が同時に同じ位置へ挿入すると `pos` が衝突しうる。その場合は `(pos, id)` の
  辞書順で決定的に並べる（片方が必ず先になり、表示が壊れない）。

---

## 3. API

すべて `Authorization: Bearer <token>` が必要。token は server の設定ファイルに書いた固定文字列
（単独ユーザーのため）。Tailscale 内限定なので、これは「同じ tailnet の別マシンから
事故で叩かれる」ことを防ぐ程度の位置付け。

| メソッド | パス | 用途 |
|---|---|---|
| `GET` | `/api/sync?since=<rev>` | 差分取得。`{rev, wants[]}` を返す。`since` 省略で全件 |
| `POST` | `/api/wants` | 作成。body に `id` を含める（冪等 upsert） |
| `PATCH` | `/api/wants/:id` | **部分更新**。送ったフィールドだけ上書き |
| `DELETE` | `/api/wants/:id` | tombstone 化 |
| `GET` | `/api/events` | SSE。`rev` が進んだことだけ通知する |
| `GET` | `/` ほか | web の静的ファイル |

「やった」は `PATCH /api/wants/:id {"done_at": "..."}`、取り消しは `{"done_at": null}`。
専用エンドポイントは作らない。

### 競合解決

**PATCH を部分更新にすることが、そのまま競合解決になっている。**

全体を PUT する設計にすると、「TUI で並び順を変えた直後に、
古いデータを持っていたブラウザが `done_at` を立てる」だけで並び順が巻き戻る。
変更したフィールドだけを送れば、この2つの操作は衝突せずに両立する。

同じフィールドを2端末が同時に変えた場合は後勝ち。単独ユーザーなので、これで十分。

### SSE の扱い

`/api/events` は差分の中身を流さず、「`rev` が N になった」とだけ送る。
受け取ったクライアントは `GET /api/sync?since=<自分のrev>` を叩き直す。

- イベントの取りこぼしが起きても、次に届いた通知で追いつける（自己修復する）。
- 再接続時の補償ロジックが要らない。
- ウィジェットを作る段階で、この SSE を FCM に差し替えるか、そのまま使うかを選べる。

---

## 4. オフライン動作

各クライアントはローカルキャッシュと未送信キュー (outbox) を持つ。

| クライアント | キャッシュ | outbox |
|---|---|---|
| TUI | `~/.config/wanna/cache.db` (SQLite) | 同 DB のテーブル |
| Web | IndexedDB | 同 |
| Android | Room | 同 |

流れ:

1. 起動時にキャッシュを表示（即座に描画される）
2. 裏で `GET /api/sync?since=<ローカルrev>` して差分を反映
3. 編集はまずローカルに適用し、outbox に積む
4. outbox を順に送信。ID がクライアント生成なので、送信失敗→再送で重複しない
5. サーバー到達不能でも、ローカルだけで通常どおり操作できる

Phase 2 の TUI は 1〜3 を先に作り、outbox は Phase 3 と一緒でもよい。

---

## 5. TUI の画面

### やりたいこと（メイン画面）

2×2 グリッドで4区分を同時に表示する。元のマトリックスの空間的な意味（上がエネルギー高、
右が clau高）をそのまま保ちながら、中身をリストにする。

```
エネルギー高
 ┌─自分でやる────┐┌─claudeとやる──┐
 │ 1 論文を読む     ││ 1 サーバ実装      │
 │ 2 部屋の片付け   ││ 2 webのUI         │
 │ 3 買い出し       ││                   │
 └──────────────────┘└───────────────────┘
 ┌─片手間────────┐┌─流し込む──────┐
 │ 1 メール返信     ││ 1 誘導文の推敲    │
 │ 2 ゴミ出し       ││                   │
 └──────────────────┘└───────────────────┘
エネルギー低      clau低 →  clau高
```

- `done_at` が入ったものはここに出ない。
- 各区分の中では `(pos, id)` の昇順。
- 行頭の番号は**その区分内での通し番号**。DB には保存せず表示時に振る
  （順位ではなく、数字キーで選ぶための目印）。
- サイドバーには選択中の名前とメモを出す。画面幅が足りないときは畳む。

### やったこと

`a` キーで切り替える別画面。達成日の降順で並べる。

```
▼ やったこと

  2026-09-18  サーバ実装         (claudeとやる)
  2026-09-15  部屋の片付け       (自分でやる)
  2026-09-11  論文を読む         (自分でやる)
  2026-09-03  買い出し           (自分でやる)
```

- 区分は当時のものをそのまま添える（記録なので変えない）。
- ここで `u` を押すと「やった」を取り消して、元の区分の末尾に戻る。
- `a` か `Esc` でやりたいことリストへ戻る。

### キーバインド

| キー | アクション |
|------|-----------|
| `n` | 追加（名前 → エネルギー 高/低 → clau度 高/低）。その区分の末尾に入る |
| `e` | 編集（名前・メモ） |
| `t` | **やった**（リストから消え、やったこと側へ回る） |
| `d` | 削除（y/n 確認） |
| `h/j/k/l` | カーソル移動。`j/k` は区分内、`h/l` は左右の区分へ |
| `K` / `J` | **選択中のものを区分内で上/下へ移動**（並べ替え） |
| `H` / `L` | 選択中のものを左/右の区分へ移動（clau度の切り替え） |
| `E` | エネルギーを切り替え（上下の区分へ移動） |
| `1`-`9`, `0` | カーソルのある区分内で、その番号を選択 |
| `a` | やったこと画面へ / から戻る |
| `u` | (やったこと画面で) やったを取り消す |
| `q` / `Esc` | 終了 |

`J/K` を並べ替えに割り当てているのは、「順番を自分で決める」が中心機能で、
最も打鍵頻度が高くなるため。区分の移動は `H/L` と `E` に分けた。

---

## 6. デプロイ

Ubuntu Server ノートPC 側:

```
~/wanna/
├── wannad               server バイナリ (クロスコンパイルか現地ビルド)
├── config.toml          bind アドレス / token / DB パス
└── data/wanna.db        SQLite
```

- systemd **user** service (`~/.config/systemd/user/wannad.service`) + `loginctl enable-linger` で
  ログインなしでも常駐させる。root 権限が要らない。
- bind は `100.x.x.x:8787`（tailscale0 のアドレス）か、`127.0.0.1` + `tailscale serve`。
  `0.0.0.0` にはしない。
- Tailscale は apt を使わず、[pkgs.tailscale.com](https://pkgs.tailscale.com/stable/#static) の
  static バイナリ tarball を展開する方法がある（既存の方針に合わせる）。
- バックアップ: `sqlite3 wanna.db ".backup"` を毎日 cron で取り、7世代ローテート。
  DB は数MB にもならないので凝る必要はない。

---

## 7. 実装フェーズ

| Phase | 内容 | 完了条件 |
|---|---|---|
| 0 | workspace + `core`（モデル・fractional index・テスト） | `cargo test` が通る |
| 1 | `server` (axum + rusqlite) + `migrate` | curl で CRUD と差分同期ができ、既存データが DB に入っている |
| 2 | `tui` | 2×2 グリッドで表示・追加・並べ替え・やったこと画面が動く |
| 3 | `web` (Vite + TS, PWA) | ブラウザで追加・並べ替えし、TUI に反映される |
| 4 | `calendar-tui` からタスク画面を削除 | カレンダー専用に戻る |
| 5 | `android` (Kotlin + Compose) | スマホで操作できる |
| 6 | Glance ウィジェット | ホーム画面に区分ごとの上位が出る |

Phase 1 の時点で SSE まで入れておく。後から足すと3クライアント全部に手を入れ直すことになる。

### 移行 (`migrate`)

1. calendar-tui の `token.json` を再利用して Google Tasks から全件取得（`showCompleted=true`）
2. `~/.config/calendar-tui/task_meta.json` と notes 末尾の旧 `[todo-meta]` 行から imp/clau を引く
3. **0〜10 を2値へ畳む**: `energy = imp >= 6`, `clau = clau >= 6`
   - メタが無いものと、既定値 5 のままのものはすべて「低」に落ちる。
     移行後に `E` / `H` / `L` で手直しする前提。件数が多いなら閾値を 5 に下げてもよい
4. リスト・期限・スタック情報は**捨てる**
5. `pos` は Google Tasks 側の `position` 順を保って振り直す（= 元の並びが追加順として残る）
6. 完了済みは `done_at` を入れて取り込む → そのまま「やったこと」に並ぶ

一度実行したら `migrate` は役目を終える。リポジトリには記録として残す。
