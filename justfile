apk := "Raging+Thunder+2+-+FREE_1.0.17_APKPure.apk"
lib := "game/librthunder2lite.so"
bin := "target/i686-unknown-linux-gnu/debug/rt2"

default:
    @just --list

# Build the i686 loader inside the devcontainer
build:
    @just _in-container cargo build

# Format, lint and test inside the devcontainer
check:
    @just _in-container cargo fmt --all
    @just _in-container cargo clippy --all-targets
    @just _in-container cargo test

# Extract the game library from the APK
extract:
    mkdir -p game
    unzip -o -j "{{apk}}" lib/x86/librthunder2lite.so -d game

# Run the loader on the host, which owns the display
run: build extract
    {{bin}} "{{apk}}" {{lib}}

# Run the loader under gdb on the host
debug: build extract
    gdb --args {{bin}} "{{apk}}" {{lib}}

clean:
    rm -rf target game

_in-container +cmd:
    #!/bin/sh
    set -eu
    if [ -e /run/.containerenv ]; then exec {{cmd}}; fi
    id=$(podman ps -q --filter label=devcontainer.local_folder={{justfile_directory()}} | head -n1)
    if [ -z "$id" ]; then echo "devcontainer for {{justfile_directory()}} is not running" >&2; exit 1; fi
    exec podman exec -w /workspaces/rt2 "$id" {{cmd}}
