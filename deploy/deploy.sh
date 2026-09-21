#!/bin/sh
# 開発機から wannad を入れ替える。web は wannad に埋め込まれているので、
# ブラウザ版もこれで一緒に更新される。
#
#   make deploy                          送り先は dynabook
#   make deploy DEPLOY_HOST=別のホスト
#   WANNA_SKIP_BACKUP=1 make deploy      DB のバックアップを取らずに進める
#
# 送ったあと応答を確かめ、応答しなければ前のバイナリに戻す。
set -eu

HOST="${1:-${DEPLOY_HOST:-dynabook}}"
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BIN="$ROOT/target/release/wannad"

if [ ! -f "$BIN" ]; then
    echo "$BIN がありません。先に make server でビルドしてください" >&2
    exit 1
fi

echo "→ $HOST へ送る ($(du -h "$BIN" | cut -f1))"
# 送り先は ~/wanna（wannad が動いている場所であって、リポジトリの clone ではない）
sent=0
scp -q "$BIN" "$HOST:wanna/wannad.new" || sent=$?
if [ "$sent" = 255 ]; then
    # ssh/scp はつながらなかったときだけ 255 を返す。~/wanna の有無とは関係ない
    echo "$HOST につなげませんでした（接続か認証の問題で、~/wanna の有無とは無関係です）" >&2
    echo "  まず ssh $HOST だけで入れるか確かめてください" >&2
    echo "  毎回パスワードを聞かれるなら、ssh-copy-id $HOST を一度やっておくと通しで動きます" >&2
    exit 1
elif [ "$sent" != 0 ]; then
    echo "$HOST の ~/wanna へ送れませんでした (scp の終了コード $sent)" >&2
    echo "  ~/wanna がまだ無いなら、$HOST でリポジトリを clone して make install-server を実行してください" >&2
    exit 1
fi

ssh "$HOST" sh -s "${WANNA_SKIP_BACKUP:-0}" <<'REMOTE'
set -eu
skip_backup="${1:-0}"
cd "$HOME/wanna"

# スキーマは新しい wannad が起動した時点で書き換わるので、その前に控えを取る
if [ "$skip_backup" = 1 ]; then
    echo "  DB のバックアップは飛ばした"
elif [ -x backup.sh ] && ./backup.sh; then
    echo "  DB をバックアップした"
else
    rm -f wannad.new
    echo "DB のバックアップに失敗しました (sqlite3 は入っていますか)。" >&2
    echo "手で控えを取るか、WANNA_SKIP_BACKUP=1 を付けて実行してください" >&2
    exit 1
fi

# 動いているバイナリは mv でしか差し替えられない (cp だと Text file busy)
if [ -f wannad ]; then cp -p wannad wannad.prev; fi
chmod 755 wannad.new
mv wannad.new wannad
systemctl --user restart wannad || true

# 設定の bind に応答が返るまで待つ。curl が無ければ systemd の状態だけ見る
bind=$(sed -n 's/^[[:space:]]*bind[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' config.toml)
i=0
while [ "$i" -lt 20 ]; do
    i=$((i + 1))
    sleep 0.5
    systemctl --user is-active --quiet wannad || continue
    if [ -n "$bind" ] && command -v curl > /dev/null 2>&1; then
        # 待っている間の接続エラーは出さない (最後まで駄目なら下で status を出す)
        curl -fs -o /dev/null "http://$bind/" || continue
        echo "  $bind が応答した"
    else
        echo "  wannad は動いている (応答は確かめていない)"
    fi
    exit 0
done

echo "新しい wannad が応答しません。前のものに戻します" >&2
systemctl --user status wannad --no-pager --lines 20 >&2 || true
if [ -f wannad.prev ]; then
    mv wannad.prev wannad
    systemctl --user restart wannad || true
    echo "戻しました" >&2
fi
exit 1
REMOTE

echo "✓ $HOST の wannad を入れ替えました"
echo "  ブラウザは wanna のタブと PWA を全部閉じてから開き直してください"
echo "  手で戻すなら: ssh $HOST 'mv ~/wanna/wannad.prev ~/wanna/wannad && systemctl --user restart wannad'"
