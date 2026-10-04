.PHONY: setup get clean analyze test build-web dist dev-server fake-dsp build-windows build-linux build-android build-ios build-macos run-windows dev-windows

# First-time project setup
setup: get
	flutter doctor

# Fetch dependencies (client, shared package, server)
get:
	flutter pub get
	cd packages/dsp_core && dart pub get
	cd server && dart pub get

# Clean build artifacts and regenerate
clean:
	flutter clean
	rm -rf dist
	$(MAKE) get

# Run static analysis
analyze:
	flutter analyze
	cd packages/dsp_core && dart analyze
	cd server && dart analyze

# Run tests
test:
	flutter test
	cd packages/dsp_core && dart test
	cd server && dart test

# ─── Web panel ───

# Build the web client (offline-capable: no CDN, no service worker)
build-web:
	flutter build web --release --no-web-resources-cdn --pwa-strategy=none --no-wasm-dry-run

# Release package for the Ubuntu server: dist/dsp-panel-<version>-linux-x64.tar.gz
dist:
	tool/build_dist.sh x64

# Run the server locally without TLS and login (after `make build-web`)
dev-server:
	cd server && dart run bin/dsp_server.dart serve --no-tls --no-auth --port 8080 --data-dir ../.dev-data --web-root ../build/web

# Simulated DSP 408 (port 19761) and DSP 206 (port 19762)
fake-dsp:
	cd server && dart run bin/fake_dsp.dart --port 19761 --models 408,206

# ─── Native apps (not adapted to the panel server; kept for reference) ───

# Build Windows release
build-windows:
	flutter build windows

# Build Linux release
build-linux:
	flutter build linux

# Build MacOS release
build-macos:
	flutter build macos

# Build Android release
build-android:
	flutter build apk

# Build iOS release. You MUST be on an Apple device for this to work.
build-ios:
	flutter build ios

# Run on Windows (debug)
run-windows:
	flutter run -d windows

# Run on Windows (debug) with hot reload
dev-windows:
	flutter run -d windows --hot
