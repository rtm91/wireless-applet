rootdir := ''
prefix := '/usr'
appid := 'io.github.rtm91.WirelessApplet'
bin := 'cosmic-applet-wireless'
target := 'release'

bindir := rootdir + prefix / 'bin'
sharedir := rootdir + prefix / 'share'
icondir := sharedir / 'icons' / 'hicolor' / 'scalable' / 'status'

default: build

# Compiles with the release profile
build:
    cargo build --release

# Compiles with the debug profile
build-debug:
    cargo build

# Installs into the system prefix (needs root)
install: build
    install -Dm0755 target/{{target}}/{{bin}} {{bindir}}/{{bin}}
    install -Dm0644 data/{{appid}}.desktop {{sharedir}}/applications/{{appid}}.desktop
    install -Dm0644 data/icons/scalable/status/{{appid}}-symbolic.svg {{icondir}}/{{appid}}-symbolic.svg

# Installs into the user's home, no root needed
install-user: build
    install -Dm0755 target/{{target}}/{{bin}} ~/.local/bin/{{bin}}
    install -Dm0644 data/{{appid}}.desktop ~/.local/share/applications/{{appid}}.desktop
    install -Dm0644 data/icons/scalable/status/{{appid}}-symbolic.svg ~/.local/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg

uninstall:
    rm -f {{bindir}}/{{bin}}
    rm -f {{sharedir}}/applications/{{appid}}.desktop
    rm -f {{icondir}}/{{appid}}-symbolic.svg

uninstall-user:
    rm -f ~/.local/bin/{{bin}}
    rm -f ~/.local/share/applications/{{appid}}.desktop
    rm -f ~/.local/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg

# Runs the applet outside the panel for quick iteration
run:
    cargo run

check:
    cargo clippy --all-targets -- -W clippy::all

fmt:
    cargo fmt
