# web を先にビルドしてから wannad に埋め込む
.PHONY: all web server tui test install-tui install-server deploy

# 入れ替え先。make deploy DEPLOY_HOST=... で変えられる
DEPLOY_HOST ?= dynabook

all: server tui

web:
	cd web && npm ci && npm run build

server: web
	cargo build --release -p wanna-server

tui:
	cargo build --release -p wanna-tui

test:
	cargo test
	cd web && npm test

# 開発機から DEPLOY_HOST の wannad を入れ替える (ブラウザ版も一緒に更新される)
deploy: server
	deploy/deploy.sh $(DEPLOY_HOST)

# Ubuntu Server 側: ~/wanna/ に配置する (初回のみ。以後は開発機から make deploy)
install-server: server
	mkdir -p $(HOME)/wanna/data
	install -m 755 target/release/wannad $(HOME)/wanna/wannad
	install -m 755 deploy/backup.sh $(HOME)/wanna/backup.sh
	test -f $(HOME)/wanna/config.toml || install -m 600 config.example.toml $(HOME)/wanna/config.toml
	mkdir -p $(HOME)/.config/systemd/user
	install -m 644 deploy/wannad.service $(HOME)/.config/systemd/user/wannad.service

# この開発機に TUI を入れる (~/.cargo/bin/wanna)。更新も同じコマンドで上書きされる
install-tui:
	cargo install --path tui --target-dir target
	@test -f $(HOME)/.config/wanna/config.toml || \
		echo "  サーバーと同期するなら tui.config.example.toml を $(HOME)/.config/wanna/config.toml に置いてください"
