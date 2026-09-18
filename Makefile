# web を先にビルドしてから wannad に埋め込む
.PHONY: all web server tui test install-server

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

# Ubuntu Server 側: ~/wanna/ に配置する
install-server: server
	mkdir -p $(HOME)/wanna/data
	install -m 755 target/release/wannad $(HOME)/wanna/wannad
	install -m 755 deploy/backup.sh $(HOME)/wanna/backup.sh
	test -f $(HOME)/wanna/config.toml || install -m 600 config.example.toml $(HOME)/wanna/config.toml
	mkdir -p $(HOME)/.config/systemd/user
	install -m 644 deploy/wannad.service $(HOME)/.config/systemd/user/wannad.service
