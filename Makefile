.PHONY: build install-deps

# Automatically triggers the installation check before running the build
build: install-deps
	wasm-pack build --release --target bundler

build-web: install-deps
	wasm-pack build --release --target web --out-dir pkg-web

# Checks if wasm-pack is installed, installs it via cargo if missing
install-deps:
	@command -v wasm-pack >/dev/null 2>&1 || { \
		echo "wasm-pack not found. Installing via cargo..."; \
		cargo install wasm-pack; \
	}
