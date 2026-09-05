# Raven Controls

Keyboard backlight, fan speeds and thermal profiles for Raven Linux — on
whatever machine it is installed on, not on one laptop.

GTK 4 + libadwaita, in Rust. No vendor SDK, no `asusctl`, no `nbfc`, no
reverse-engineered blob. Everything here drives interfaces that are documented
in the Linux kernel tree.

```
imlazy          # build the window and the daemon
imlazy run      # open the window
imlazy probe    # what this machine exposes, and why it does not expose the rest
imlazy test     # 102 tests, eleven of them against captured machines
imlazy check    # fmt, clippy and the tests -- what has to pass before a commit
imlazy install  # to /usr/local
```

`imlazy list` prints every command, `imlazy -i` picks one interactively. The
Makefile forwards to the same commands for anyone who types `make` out of
habit — `lazy.toml` is the one definition, so the two cannot drift.

## The problem this is built around

It takes about fifteen lines to control the keyboard light on a ROG Zephyrus.
Those fifteen lines work on a ROG Zephyrus.

The same is true of every other laptop, which is why there are a dozen
single-vendor tools and no general one. Write against `asus-nb-wmi` and you
have an ASUS tool; write against `/proc/acpi/ibm/fan` and you have a ThinkPad
tool. Neither survives contact with the next machine.

**So nothing here asks what laptop it is running on.** It asks what interfaces
the kernel exposes, because the kernel has already done the per-vendor work and
published one name for the result:

| What | Where the kernel puts it | Covers |
|---|---|---|
| Keyboard backlight | `/sys/class/leds/*::kbd_backlight` | asus, dell, tpacpi, smc, hp, msi, samsung, system76, chromeos — every driver that has one |
| RGB keyboard | the same, plus `multi_index` / `multi_intensity` | anything using the multicolour LED class |
| Fan duty and speed | `/sys/class/hwmon/*/pwm*`, `fan*_input` | nct6775, it87, amdgpu, nouveau, dell-smm, thinkpad_acpi, applesmc, asus-ec-sensors |
| Coarse thermal modes | `/sys/firmware/acpi/platform_profile` | ASUS, Lenovo, HP, Dell, Framework, MSI, Surface |
| ACPI fans | `/sys/class/thermal/cooling_device*` | machines with `PNP0C0B` or `INT3404` and nothing else |
| Backlight without root | `org.freedesktop.UPower.KbdBacklight` | any distribution running UPower |

Grep the source for a vendor name and you will find them in two places: a table
of documented sysfs attributes in `fan/vendor.rs`, and a list of kernel modules
in `diagnose.rs`. Both are **data**. Adding a machine is a row, not a provider,
and never a UI change.

## The look

The same glass shell as Raven Settings, Store and Power: the shared palette as
libadwaita named colours, a translucent window for Huginn to blur behind, the
sidebar, and cards. `src/ui/theme.rs` carries the palette and says which files
it is in lockstep with.

Appearance comes from `~/.config/desktop.toml` — the file Raven Settings'
Appearance page writes — so the accent, light/dark and the transparency switch
follow the desktop rather than being decided here. It is re-read on the refresh
tick, so changing the accent in Settings recolours this window, the fan-curve
graph included, without a restart. Only the three fields it acts on are
modelled, so a key another application adds to that file cannot break the parse.

The **sections in the sidebar are discovered like everything else**: a desktop
gets no Keyboard entry, a machine with no sensors gets no Sensors entry, and a
laptop with no fan interface gets a Fans page carrying the diagnosis rather than
an empty one.

## How that shape falls out

```
crates/raven-hw         what this machine can do. No GTK, no daemon, no root.
crates/raven-controlsd  the privileged half: owns fan writes, runs curves, and
                        hands the fans back when it dies.
crates/raven-controls   the window.
```

Each provider publishes `Knob`s described by a **domain** — a percentage,
ordered steps, named modes, a switch, a colour — and the window renders
domains. Four widgets cover every fan and backlight interface in the kernel, so
a laptop with four fans and an RGB keyboard and a laptop with one two-level
light run the same code and get windows that fit them.

## Testing machines nobody here owns

Every path in `raven-hw` goes through a `Root`, so discovery can be pointed at a
captured `/sys` instead of the live one. `crates/raven-hw/tests/fixtures` holds
ten of them and `tests/machines.rs` asserts against each:

| Fixture | What it pins down |
|---|---|
| `zephyrus-gu502du` | four-step light, ten decoy LEDs, and no fan interface at all |
| `thinkpad-t14` | LED light, hwmon PWM, platform profile and procfs fan at once |
| `desktop-nct6798` | five headers in Smart Fan IV (`pwm_enable = 5`) plus a Radeon |
| `dual-gpu` | two chips with the same driver name |
| `rgb-keyboard` | the multicolour LED class |
| `macbook-applesmc` | fan speeds with nothing to drive |
| `acpi-fan` | `cur_state` fans among cooling devices that are throttles |
| `multi-handler-profile` | the kernel 6.14 platform-profile layout and its alias |
| `bare-machine` | nothing found, which is a result and not a failure |
| `zephyrus-with-module` | what the diagnosis promises, actually delivered |
| `asus-enable-only` | `pwmN_enable` with no `pwmN` — captured from real hardware |

Three real bugs came out of this, none of them visible on the machine as it
stood:

- `fan1_input` and `temp1_input` were never matched — only a bare `fanN` would
  have been — so any MacBook would have reported no fans at all.
- `pwm1_enable`'s driver-specific automatic modes were flattened onto the
  generic `2`, which would have stranded most desktop boards on a mode they
  never used.
- **`pwmN_enable` with no `pwmN`.** `asus_wmi` can hand each fan between
  firmware and full speed but cannot set a duty, so it publishes two enables and
  no duty attribute. The mode knob was only ever built inside the loop over duty
  channels, so both controls were silently dropped. That one took real hardware:
  it appeared the moment `asus_nb_wmi` was loaded on the development machine,
  and it is now `asus-enable-only` in the table above.

That last one also needed a safety decision. "Manual" on a driver with no duty
attribute means taking the fan off firmware and leaving it wherever the embedded
controller happened to leave it, with nothing able to move it again — so it is
kept off the menu, while still being displayed if a driver is already sitting in
it.

If RavenControls gets your machine wrong:

```
imlazy capture capture_dir=~/machine
```

That writes the subset of `/sys`, `/proc` and the kernel config it reads — LED
and hwmon attributes, thermal zones, DMI identification, the module list; no
serial numbers, no UUIDs, no user data, and the list is at the top of
`capture.rs` to read before sending anything. Drop the directory into
`tests/fixtures/`, add a test, and that machine is regression-tested forever by
people who do not own one.

## When there is nothing to show

`imlazy probe` on the laptop this was written on:

```
ASUSTeK COMPUTER INC. Zephyrus G GU502DU_GA502DU, Linux 6.17.11-raven

== controls
  [KeyboardBacklight] Keyboard backlight
      domain   Percent { raw_max: 3 }
      writable false

== why there is no fan control here
  asus_nb_wmi would add platform profile, throttle policy, fan boost — it is
  built for this kernel but not loaded:
      sudo modprobe asus_nb_wmi
  asus_ec_sensors would add fan speeds and temperatures, but this kernel was
  built without CONFIG_SENSORS_ASUS_EC.
```

An empty window that says "not supported" tells its owner nothing about whether
the machine cannot do it or whether a module is simply not loaded. This reads
`/proc/modules` and the kernel config — including `/proc/config.gz` — and tells
the three cases apart: built and not loaded (here is the line to type), never
compiled (no amount of modprobe will help), or loaded already and still nothing
(the firmware is the limit; stop chasing it).

## Fans, and why there is a daemon

Setting a fan speed means writing `pwmN_enable=1`, and from that moment the
firmware's thermal management is off. If whatever did that stops running —
crashes, is killed, is upgraded, or is a settings window somebody closed — the
fan stays exactly where it was left. At 0% under load that is a thermal
shutdown at best.

So `raven-controlsd` owns fan writes, and:

- records each channel's original `pwmN_enable` and restores it on `Drop`,
  which covers returns and panics;
- installs SIGTERM/SIGINT/SIGHUP handlers, because `Drop` does not run for a
  signal, and `raven-rc stop` and Ctrl-C are both signals;
- runs a **watchdog** on its own control loop — a tick that has not completed in
  20 seconds hands every channel back to firmware, without needing the lock the
  stuck thread is holding;
- **re-asserts** `pwmN_enable=1` every tick, because firmware silently takes the
  channel back across suspend/resume;
- goes to **full duty** and says so when a sensor passes the temperature its own
  driver calls critical — the curve stops being the authority there;
- **verifies every write by reading it back**, because plenty of embedded
  controllers accept a PWM write, return success, and change nothing.

The window refuses to take manual fan control when the daemon is not running,
and says why. The shipped udev rule deliberately grants the keyboard light and
**not** the fans; `data/90-raven-controls.rules` explains that at length, and a
test asserts nobody has helpfully added it.

## Fan curves

Curves live in the daemon, persist to `/var/lib/raven-controls/state.json`, and
are re-applied at start — a curve is a setting, not a session. The editor is a
graph you drag, with the live temperature drawn on it.

The curve engine is pure and unit-tested, and it earns that by getting three
things right that a plain interpolation gets wrong:

- **Oscillation.** A CPU ticking between 61 and 62 degrees makes a naive
  controller audibly surge once a second. Temperature is followed up
  immediately and down only after it has fallen past a hysteresis band.
- **Stall.** Most fans will not start from rest below about a fifth of full
  duty; commanded to 8% they sit still while the curve believes it is cooling.
  Non-zero outputs are raised to a floor. Zero stays zero, so a fan can stop.
- **Extrapolation.** A curve drawn between 40 and 90 degrees says nothing about
  120, and a linear extension would confidently answer 160%. It is flat outside
  its endpoints.

## On a Raven image

RavenControls is built into the image rather than installed onto it. In
[RavenLinux](../RavenLinux):

| Where | What |
|---|---|
| `packages/gui/raven-controls/package.toml` | the package: both binaries, the icon, the metainfo, the udev rule |
| `scripts/stages/stage-gui.sh` | `stage_controls()` builds `--workspace`, stages both binaries, the udev rule and the icon |
| `scripts/stages/stage-gui.sh` | `install_desktop_entries()` writes the launcher entry — every entry on the image is decided in that one function |
| `etc/raven/init.toml` | the `controlsd` service, so raven-init starts the daemon at boot |
| `configs/raven/services/controlsd.toml` | the same service as a drop-in, for a machine installed before it existed |

`raven-rc` drives it like any other service:

```bash
raven-rc status controlsd
raven-rc restart controlsd
raven-rc stop controlsd     # every fan goes back to firmware on the way out
```

`CONTROLS_SKIP=1` builds an image without it. The `controlsd` service stays
defined in that case and raven-init logs that its binary is missing, the same
way it treats an absent `raven-powerd`.

### It shares the platform profile with raven-powerd

`raven-powerd` writes `/sys/firmware/acpi/platform_profile` itself, on every
power-supply change and on a timer, as part of the `[profile]` presets in
`/etc/raven/power.toml` (`manage = true` by default). Two things writing one
attribute is exactly the situation where a settings window lies to you: set the
profile here, and it is undone within seconds with no explanation.

So when `/run/raven-power/profile` exists — raven-powerd's marker for "I am
managing profiles, and this is the preset I last applied" — that row says who
owns it and how to take it back:

> raven-powerd is managing this (last applied: balanced) and re-applies it when
> the power supply changes. A change here will not stick. Hold one preset for
> the session with `profile <preset>` on /run/raven-power/ctl, or set
> `manage = false` under `[profile]` in /etc/raven/power.toml.

Only the ACPI platform profile is contested. Vendor throttle policies, hwmon
channels and the keyboard light are RavenControls' alone.

## Installing by hand

```bash
imlazy install      # binaries, launcher entry, icon, udev rule, and the service
imlazy uninstall    # all of it back out, service stopped first
```

One command each. `install` also installs the udev rule that lets the `video`
group set the keyboard backlight, and — on a machine with `raven-rc` — the
`controlsd` service, reloading udev and starting the daemon. On a distribution
that is not Raven the service step is skipped and it says so.

`uninstall` stops the daemon *before* removing its service file, so
raven-controlsd hands every fan back to firmware on the way out rather than
being deleted out from under itself. A test asserts that order, and another
asserts that every path `install` writes is a path `uninstall` removes.

The pieces are separately callable when the whole is not wanted:

```bash
imlazy install-udev      # only the keyboard backlight rule
imlazy install-service   # only the daemon under raven-init
```

### Packaging, and testing the install itself

Three variables make this stageable and, more usefully, testable:

```bash
imlazy install prefix=/tmp/root sysconfdir=/tmp/root/etc sudo=
```

`sudo=` is empty, so it writes as you; `prefix` and `sysconfdir` put everything
under a temporary root. Nothing reaches outside those two, and the steps that
touch a *running* system — `udevadm control --reload`, `raven-rc start` — are
skipped unless `sysconfdir` really is `/etc`, so staging into a temporary root
cannot reload the init of the machine doing the staging.

`video` is the group the Raven session already holds for DRM, so none of this
adds a group or grants anything a logged-in user did not already have.

## Licence

GPL-3.0-or-later.
