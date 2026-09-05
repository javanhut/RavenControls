# RavenControls. `make`, `make run`, `make probe`, `sudo make install`.
# lazy.toml mirrors these for imlazy; keep the two in step.

APP_ID    := com.ravencontrols.Raven
BIN_NAME  := raven-controls
DAEMON    := raven-controlsd
PREFIX    ?= /usr/local
BINDIR    := $(PREFIX)/bin
DATADIR   := $(PREFIX)/share
ICONDIR   := $(DATADIR)/icons/hicolor/scalable/apps
DESTDIR   ?=
PROFILE   ?= release
CARGO_FLAGS := $(if $(filter release,$(PROFILE)),--release,)
TARGET_DIR  := target/$(PROFILE)

.PHONY: all build run probe daemon capture fixtures test check clean install uninstall install-udev

all: build

build:
	cargo build --locked --workspace $(CARGO_FLAGS)

run:
	cargo run $(CARGO_FLAGS) -p raven-controls

# What this machine exposes, and -- when that is nothing -- why. The first
# thing to run on hardware RavenControls gets wrong, and the thing to paste
# into a bug report. Needs no display, no daemon and no root.
probe:
	cargo run -q $(CARGO_FLAGS) -p raven-controls -- --probe

# The daemon, in the foreground, for watching what it does.
daemon: build
	sudo $(TARGET_DIR)/$(DAEMON)

# Turn this machine into a test fixture. See CONTRIBUTING in the README.
capture:
	cargo run -q $(CARGO_FLAGS) -p raven-controls -- --capture $(or $(DIR),./machine)

fixtures:
	./scripts/make-fixtures.sh

test:
	cargo test --locked --workspace

check:
	cargo fmt --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	cargo test --locked --workspace

clean:
	cargo clean

define update-caches
	@if [ -z "$(DESTDIR)" ]; then \
		command -v update-desktop-database >/dev/null 2>&1 && \
			update-desktop-database -q "$(DATADIR)/applications" || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && \
			gtk-update-icon-cache -qtf "$(DATADIR)/icons/hicolor" || true; \
	fi
endef

install: build
	install -Dm755 "$(TARGET_DIR)/$(BIN_NAME)" "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	install -Dm755 "$(TARGET_DIR)/$(DAEMON)" "$(DESTDIR)$(BINDIR)/$(DAEMON)"
	install -Dm644 "data/$(APP_ID).desktop" "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	install -Dm644 "data/icons/hicolor/scalable/apps/$(APP_ID).svg" "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	install -Dm644 "data/90-raven-controls.rules" "$(DESTDIR)$(DATADIR)/$(BIN_NAME)/90-raven-controls.rules"
	install -Dm644 "data/controlsd.toml" "$(DESTDIR)$(DATADIR)/$(BIN_NAME)/controlsd.toml"
	$(update-caches)
	@echo
	@echo "Installed. Two optional steps, and the window will tell you if you skip them:"
	@echo "  make install-udev                     keyboard backlight without root"
	@echo "  sudo cp data/controlsd.toml /etc/raven/init.d/ && sudo raven-rc reload && sudo raven-rc start controlsd"
	@echo "                                        fan curves and manual fan speeds"

# Separate from install because it writes outside the prefix, and because a
# packager wants to place it themselves.
install-udev:
	install -Dm644 "data/90-raven-controls.rules" "$(DESTDIR)/etc/udev/rules.d/90-raven-controls.rules"
	@if [ -z "$(DESTDIR)" ]; then \
		udevadm control --reload && udevadm trigger -s leds; \
		echo "udev rule installed and applied."; \
	fi

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(DAEMON)"
	rm -f "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	rm -f "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	rm -rf "$(DESTDIR)$(DATADIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)/etc/udev/rules.d/90-raven-controls.rules"
	$(update-caches)
