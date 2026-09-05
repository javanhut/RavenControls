# RavenControls is built with imlazy. lazy.toml is the interface, and this file
# forwards to it -- so `make install` and `imlazy install` cannot drift, because
# there is only one definition of either.
#
#   imlazy list     every command, with descriptions
#   imlazy -i       pick one interactively
#
# RavenLinux packages this with `[build] system = "cargo"` and an explicit
# `[install] files` list (see RavenLinux/packages/), so neither this file nor
# lazy.toml is on the packaging path.

LAZY ?= imlazy

# Every target below forwards to the imlazy command of the same name, and a test
# (crates/raven-hw/tests/packaging.rs) fails if one of them stops existing.
.DEFAULT_GOAL := build
.PHONY: build run probe daemon capture fixtures test check fmt clean \
        install install-udev install-service uninstall

build run probe daemon capture fixtures test check fmt clean \
install install-udev install-service uninstall:
	@command -v $(LAZY) >/dev/null 2>&1 || { \
		echo "imlazy is not installed, and it is how this project builds."; \
		echo "For a one-off without it:  cargo build --locked --workspace --release"; \
		exit 1; \
	}
	@$(LAZY) $@
