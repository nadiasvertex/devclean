# List recipes
default:
    @just --list

# Build a release binary
build:
    cargo build --release

# Build and install the release binary to ~/bin
install: build
    mkdir -p ~/bin
    install -m 755 target/release/devclean ~/bin/devclean
