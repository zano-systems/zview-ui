set positional-arguments

# Run the native UI with a decoded JSON document, or the bundled sample.
ui file="":
    if [ -n "$1" ]; then cargo run -- "$1"; else cargo run; fi

# Build and open the browser UI. Ctrl+C stops the local-only server.
web-ui file="":
    python3 scripts/web_ui.py serve "$1"

# Generate a standalone static site in target/web (no user data included).
web-build:
    python3 scripts/web_ui.py build --release

# Install the browser build tools; wasm-bindgen stays inside target/tools.
web-setup:
    rustup toolchain install nightly --profile minimal --target wasm32-unknown-unknown
    cargo install wasm-bindgen-cli --version 0.2.129 --locked --root target/tools
