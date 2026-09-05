#!/bin/sh
# Build the captured sysfs trees that crates/raven-hw/tests/machines.rs runs
# against.
#
# RavenControls claims to work on laptops none of us owns. That claim is only
# worth anything if it is tested, and it can only be tested if discovery runs
# against a machine's /sys rather than this one's -- which is what Root::at is
# for. These trees are small, hand-written from the kernel's own ABI
# documentation and from the driver sources, and each one exercises a shape the
# providers have to get right.
#
# A real machine is captured with `raven-controls --capture DIR`, which writes
# the same layout. Dropping one in here and adding a test is how a machine
# nobody here has ever seen stops regressing.
set -eu

here=$(dirname "$0")
fixtures="$here/../crates/raven-hw/tests/fixtures"
rm -rf "$fixtures"
mkdir -p "$fixtures"

# put <path> <contents...>
put() {
    path="$1"; shift
    mkdir -p "$(dirname "$path")"
    printf '%s\n' "$*" > "$path"
}

dmi() {
    put "$1/sys/class/dmi/id/sys_vendor" "$2"
    put "$1/sys/class/dmi/id/product_name" "$3"
    put "$1/proc/sys/kernel/osrelease" "${4:-6.12.0}"
}

# ---------------------------------------------------------------------------
# 1. ROG Zephyrus G15 GU502DU. Captured from the machine this was written on.
#
#    The awkward case, and the reason the diagnosis code exists: a keyboard
#    light with four steps and no fan interface of any kind, because asus_nb_wmi
#    is built and not loaded and CONFIG_SENSORS_ASUS_EC was never turned on.
# ---------------------------------------------------------------------------
m="$fixtures/zephyrus-gu502du"
dmi "$m" "ASUSTeK COMPUTER INC." "Zephyrus G GU502DU_GA502DU" "6.17.11-raven"
put "$m/sys/class/leds/asus::kbd_backlight/brightness" 0
put "$m/sys/class/leds/asus::kbd_backlight/max_brightness" 3
put "$m/sys/class/leds/asus::kbd_backlight/trigger" "[none] rfkill0 timer"
# The keyboard's other LEDs, which must not be mistaken for the backlight.
for led in capslock numlock scrolllock compose kana; do
    put "$m/sys/class/leds/input21::$led/brightness" 0
    put "$m/sys/class/leds/input21::$led/max_brightness" 1
done
put "$m/sys/class/leds/rtw88-0000:03:00.0/brightness" 0
put "$m/sys/class/leds/rtw88-0000:03:00.0/max_brightness" 1
# hwmon with temperatures and no PWM at all.
put "$m/sys/class/hwmon/hwmon1/name" acpitz
put "$m/sys/class/hwmon/hwmon1/temp1_input" 47000
put "$m/sys/class/hwmon/hwmon3/name" amdgpu
put "$m/sys/class/hwmon/hwmon3/temp1_input" 44000
put "$m/sys/class/hwmon/hwmon3/temp1_label" edge
# Eight CPU throttles in the thermal class, and not one fan.
i=0; while [ $i -lt 8 ]; do
    put "$m/sys/class/thermal/cooling_device$i/type" Processor
    put "$m/sys/class/thermal/cooling_device$i/cur_state" 0
    put "$m/sys/class/thermal/cooling_device$i/max_state" 3
    i=$((i + 1))
done
put "$m/proc/modules" "asus_wmi 90112 1 hid_asus, Live 0x0000000000000000
hid_asus 32768 0 - Live 0x0000000000000000
platform_profile 20480 1 asus_wmi, Live 0x0000000000000000"
put "$m/boot/config-6.17.11-raven" "CONFIG_ASUS_NB_WMI=m
CONFIG_ASUS_WMI=m
# CONFIG_SENSORS_ASUS_EC is not set
# CONFIG_SENSORS_ASUS_WMI is not set
CONFIG_HID_ASUS=m
CONFIG_LEDS_CLASS_MULTICOLOR=y"

# ---------------------------------------------------------------------------
# 2. ThinkPad T14. Every path at once: an LED-class light with two levels, a
#    hwmon PWM, an ACPI platform profile, and thinkpad-acpi's procfs fan.
# ---------------------------------------------------------------------------
m="$fixtures/thinkpad-t14"
dmi "$m" "LENOVO" "20UD0013UK" "6.12.0"
put "$m/sys/class/leds/tpacpi::kbd_backlight/brightness" 1
put "$m/sys/class/leds/tpacpi::kbd_backlight/max_brightness" 2
put "$m/sys/class/hwmon/hwmon5/name" thinkpad
put "$m/sys/class/hwmon/hwmon5/pwm1" 128
put "$m/sys/class/hwmon/hwmon5/pwm1_enable" 2
put "$m/sys/class/hwmon/hwmon5/fan1_input" 3100
put "$m/sys/class/hwmon/hwmon5/fan1_label" "CPU fan"
put "$m/sys/class/hwmon/hwmon5/temp1_input" 52000
put "$m/sys/class/hwmon/hwmon5/temp1_label" CPU
put "$m/sys/class/hwmon/hwmon5/temp1_crit" 100000
put "$m/sys/firmware/acpi/platform_profile" balanced
put "$m/sys/firmware/acpi/platform_profile_choices" "low-power balanced performance"
mkdir -p "$m/proc/acpi/ibm"
printf 'status:\t\tenabled\nspeed:\t\t3100\nlevel:\t\tauto\n' > "$m/proc/acpi/ibm/fan"
put "$m/proc/modules" "thinkpad_acpi 118784 0 - Live 0x0000000000000000"

# ---------------------------------------------------------------------------
# 3. Desktop, ASUS board, NCT6798 Super-I/O and a discrete Radeon.
#
#    No keyboard light -- desktops mostly have none -- five fan headers, and a
#    Super-I/O sitting in Smart Fan IV, which is pwmN_enable = 5. A control that
#    only understood 0/1/2 would strand this board on a mode it never used.
# ---------------------------------------------------------------------------
m="$fixtures/desktop-nct6798"
dmi "$m" "ASUS" "ROG STRIX B550-F GAMING" "6.12.0"
put "$m/sys/class/hwmon/hwmon2/name" nct6798
n=1; while [ $n -le 5 ]; do
    put "$m/sys/class/hwmon/hwmon2/pwm$n" $((40 * n))
    put "$m/sys/class/hwmon/hwmon2/pwm${n}_enable" 5
    put "$m/sys/class/hwmon/hwmon2/fan${n}_input" $((600 * n))
    n=$((n + 1))
done
put "$m/sys/class/hwmon/hwmon2/fan1_label" "CPU fan"
put "$m/sys/class/hwmon/hwmon2/temp1_input" 41000
put "$m/sys/class/hwmon/hwmon2/temp1_label" SYSTIN
put "$m/sys/class/hwmon/hwmon2/temp1_crit" 95000
put "$m/sys/class/hwmon/hwmon4/name" amdgpu
put "$m/sys/class/hwmon/hwmon4/pwm1" 90
put "$m/sys/class/hwmon/hwmon4/pwm1_enable" 2
put "$m/sys/class/hwmon/hwmon4/fan1_input" 1400
put "$m/sys/class/hwmon/hwmon4/temp1_input" 55000
put "$m/sys/class/hwmon/hwmon4/temp1_label" edge
put "$m/sys/class/hwmon/hwmon4/temp1_crit" 100000
put "$m/proc/modules" "nct6775 61440 0 - Live 0x0000000000000000"

# ---------------------------------------------------------------------------
# 4. Two identical GPUs. Not a laptop, but the case that breaks any scheme
#    keying controls on the driver name alone.
# ---------------------------------------------------------------------------
m="$fixtures/dual-gpu"
dmi "$m" "Supermicro" "X11SPA-T" "6.12.0"
for slot in 0000:01:00.0 0000:41:00.0; do
    idx=$(echo "$slot" | cut -c6-7)
    d="$m/sys/class/hwmon/hwmon$idx"
    put "$d/name" amdgpu
    put "$d/pwm1" 100
    put "$d/pwm1_enable" 2
    put "$d/fan1_input" 1200
    put "$d/temp1_input" 60000
    mkdir -p "$m/sys/devices/pci/$slot"
    ln -sf "../../../devices/pci/$slot" "$d/device"
done

# ---------------------------------------------------------------------------
# 5. A laptop with an RGB keyboard driven through the kernel's multicolour LED
#    class -- brightness and multi_intensity as separate controls, which is
#    exactly how the framework models it.
# ---------------------------------------------------------------------------
m="$fixtures/rgb-keyboard"
dmi "$m" "System76" "oryp9" "6.12.0"
put "$m/sys/class/leds/rgb:kbd_backlight/brightness" 200
put "$m/sys/class/leds/rgb:kbd_backlight/max_brightness" 255
put "$m/sys/class/leds/rgb:kbd_backlight/multi_index" "red green blue"
put "$m/sys/class/leds/rgb:kbd_backlight/multi_intensity" "255 128 0"
put "$m/sys/class/hwmon/hwmon6/name" system76_acpi
put "$m/sys/class/hwmon/hwmon6/pwm1" 60
put "$m/sys/class/hwmon/hwmon6/pwm1_enable" 2
put "$m/sys/class/hwmon/hwmon6/fan1_input" 2200
put "$m/sys/class/hwmon/hwmon6/fan1_label" "CPU fan"
put "$m/sys/class/hwmon/hwmon6/temp1_input" 48000

# ---------------------------------------------------------------------------
# 6. MacBook. applesmc reports fans and temperatures and exposes no PWM at all,
#    so this is the "readings but nothing to drive" shape.
# ---------------------------------------------------------------------------
m="$fixtures/macbook-applesmc"
dmi "$m" "Apple Inc." "MacBookPro14,1" "6.12.0"
put "$m/sys/class/leds/smc::kbd_backlight/brightness" 128
put "$m/sys/class/leds/smc::kbd_backlight/max_brightness" 255
put "$m/sys/class/hwmon/hwmon1/name" applesmc
put "$m/sys/class/hwmon/hwmon1/fan1_input" 2160
put "$m/sys/class/hwmon/hwmon1/fan1_label" "Exhaust"
put "$m/sys/class/hwmon/hwmon1/temp1_input" 46000
put "$m/proc/modules" "applesmc 32768 0 - Live 0x0000000000000000"

# ---------------------------------------------------------------------------
# 7. A machine with nothing. Not a failure to handle gracefully -- the case the
#    diagnosis exists for.
# ---------------------------------------------------------------------------
m="$fixtures/bare-machine"
dmi "$m" "Dell Inc." "Latitude 5420" "6.12.0"
put "$m/sys/class/hwmon/hwmon0/name" acpitz
put "$m/sys/class/hwmon/hwmon0/temp1_input" 44000
put "$m/proc/modules" "kvm 1000 0 - Live 0x0000000000000000"
put "$m/boot/config-6.12.0" "CONFIG_SENSORS_DELL_SMM=m
# CONFIG_SENSORS_NCT6775 is not set"

# ---------------------------------------------------------------------------
# 8. An ACPI-only fan: cur_state / max_state and no hwmon, alongside a pile of
#    cooling devices that are throttles and must be left alone.
# ---------------------------------------------------------------------------
m="$fixtures/acpi-fan"
dmi "$m" "Microsoft Corporation" "Surface Pro 7" "6.12.0"
put "$m/sys/class/thermal/cooling_device0/type" "Processor"
put "$m/sys/class/thermal/cooling_device0/cur_state" 0
put "$m/sys/class/thermal/cooling_device0/max_state" 10
put "$m/sys/class/thermal/cooling_device1/type" "INT3404 Fan"
put "$m/sys/class/thermal/cooling_device1/cur_state" 3
put "$m/sys/class/thermal/cooling_device1/max_state" 9
put "$m/sys/class/thermal/cooling_device1/fan_speed_rpm" 2800
put "$m/sys/class/thermal/cooling_device2/type" "LCD"
put "$m/sys/class/thermal/cooling_device2/cur_state" 0
put "$m/sys/class/thermal/cooling_device2/max_state" 9

# ---------------------------------------------------------------------------
# 9. The kernel 6.14+ multi-handler platform profile layout, which lives in a
#    class directory rather than in /sys/firmware/acpi.
# ---------------------------------------------------------------------------
m="$fixtures/multi-handler-profile"
dmi "$m" "Framework" "Laptop 13" "6.15.0"
put "$m/sys/class/platform-profile/platform-profile-0/name" framework_laptop
put "$m/sys/class/platform-profile/platform-profile-0/profile" balanced
put "$m/sys/class/platform-profile/platform-profile-0/choices" "low-power balanced performance"
# The compatibility alias, which is the same handler seen twice and must not
# turn into a second control.
put "$m/sys/firmware/acpi/platform_profile" balanced
put "$m/sys/firmware/acpi/platform_profile_choices" "low-power balanced performance"

# ---------------------------------------------------------------------------
# 10. ASUS with asus_nb_wmi actually loaded: what the Zephyrus above would look
#     like after `modprobe asus_nb_wmi`, which is what the diagnosis tells its
#     owner to type.
# ---------------------------------------------------------------------------
m="$fixtures/zephyrus-with-module"
dmi "$m" "ASUSTeK COMPUTER INC." "Zephyrus G GU502DU_GA502DU" "6.17.11-raven"
put "$m/sys/class/leds/asus::kbd_backlight/brightness" 2
put "$m/sys/class/leds/asus::kbd_backlight/max_brightness" 3
put "$m/sys/devices/platform/asus-nb-wmi/throttle_thermal_policy" 0
put "$m/sys/devices/platform/asus-nb-wmi/fan_boost_mode" 0
put "$m/sys/firmware/acpi/platform_profile" balanced
put "$m/sys/firmware/acpi/platform_profile_choices" "quiet balanced performance"
put "$m/proc/modules" "asus_nb_wmi 20480 0 - Live 0x0000000000000000"

echo "fixtures written to $fixtures"
ls "$fixtures"
