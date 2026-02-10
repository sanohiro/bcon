.PHONY: build run shell test clean

# Dockerイメージをビルド
build:
	docker compose build

# コンテナ内でビルド
build-docker:
	docker compose run --rm bcon cargo build --release

# コンテナ内で実行
run:
	docker compose run --rm bcon cargo run --release

# コンテナ内でシェルを起動
shell:
	docker compose run --rm bcon bash

# テストモードで実行
test:
	docker compose run --rm bcon cargo run --release -- --test

# クリーンアップ
clean:
	docker compose down -v
	docker rmi bcon-bcon 2>/dev/null || true

# ウォッチモード（ファイル変更時に自動ビルド）
watch:
	docker compose run --rm bcon cargo watch -x 'build --release'

# clippy
lint:
	docker compose run --rm bcon cargo clippy --release

# フォーマット
fmt:
	docker compose run --rm bcon cargo fmt
