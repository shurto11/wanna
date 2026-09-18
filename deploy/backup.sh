#!/bin/sh
# wanna.db を毎日バックアップし、7世代残す。crontab 例:
#   15 4 * * * $HOME/wanna/backup.sh
set -eu
DIR="${WANNA_HOME:-$HOME/wanna}"
DEST="$DIR/backup"
mkdir -p "$DEST"
sqlite3 "$DIR/data/wanna.db" ".backup '$DEST/wanna-$(date +%Y%m%d).db'"
ls -1t "$DEST"/wanna-*.db | tail -n +8 | xargs -r rm --
