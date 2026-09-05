//! The window.
//!
//! There is no ASUS page and no ThinkPad page. Rows are built from `Domain`: a
//! percentage becomes a slider, ordered steps and named modes become a
//! dropdown, a switch becomes a switch, a colour becomes a colour button. A
//! machine with four fans and an RGB keyboard and a machine with one two-level
//! light and nothing else run the same code and get windows that fit them.
//!
//! The shell is Raven's -- the glass window, sidebar and cards from Settings,
//! Store and Power, drawn by `theme.rs`. The *sections* in that sidebar are
//! discovered like everything else: a desktop with no keyboard light does not
//! get a Keyboard entry, and a laptop with no fan interface gets a Fans page
//! that explains itself instead of an empty one.
//!
//! Rows are rebuilt only when the set of controls changes -- a module loading,
//! a GPU waking up -- and their values refreshed in place every two seconds
//! otherwise, so a slider does not jump out from under a finger.

pub mod curve;
pub mod theme;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;

use raven_hw::model::{percent_to_raw, raw_to_percent, Domain, Knob, Role, Setting};

use crate::client::{Client, Snapshot, Via};
use crate::config;

const APP_ID: &str = "com.ravencontrols.Raven";
const REFRESH: Duration = Duration::from_secs(2);

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| theme::load_base());
    app.connect_activate(build);
    // The window takes no arguments; without this GTK treats `--probe` as a
    // file to open and exits.
    app.run_with_args::<&str>(&[])
}

/// A control's widgets, kept so a refresh can update values without rebuilding.
enum RowWidget {
    Scale {
        scale: gtk::Scale,
    },
    Switch(adw::SwitchRow),
    Combo {
        row: adw::ComboRow,
        options: Vec<String>,
    },
    Color {
        button: gtk::ColorDialogButton,
        channels: Vec<String>,
        raw_max: u32,
    },
    Channels {
        scales: Vec<gtk::Scale>,
    },
}

/// One entry in the sidebar, and the page it shows.
struct Section {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    /// The scrolled window in the stack, kept so a rebuild can remove it.
    page: gtk::Widget,
}

struct Window {
    client: Client,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    stack: gtk::Stack,
    nav: gtk::ListBox,
    /// The sections currently in the sidebar, in display order.
    sections: RefCell<Vec<Section>>,
    rows: RefCell<HashMap<String, RowWidget>>,
    readings: RefCell<HashMap<String, adw::ActionRow>>,
    fan_subtitles: RefCell<HashMap<String, adw::ActionRow>>,
    /// The knob-id signature the current rows were built from.
    signature: RefCell<String>,
    /// True while values are being written into widgets, so the handlers those
    /// writes fire do not turn a refresh into a hardware write.
    syncing: Cell<bool>,
    /// What was last sent for each knob, so dragging a slider across a coarse
    /// control does not write the same byte forty times.
    sent: RefCell<HashMap<String, Setting>>,
    live: RefCell<Option<Rc<curve::CurveEditor>>>,
    live_sensor: RefCell<Option<String>>,
    /// The appearance last applied, so a change in Raven Settings is noticed
    /// without restarting.
    appearance: RefCell<config::Appearance>,
}

fn build(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Controls")
        .default_width(940)
        .default_height(720)
        .build();
    window.add_css_class("raven");

    let appearance = config::appearance();
    theme::apply(&window, &appearance);

    // ---- sidebar --------------------------------------------------------
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 10);
    sidebar.add_css_class("sidebar");
    sidebar.set_size_request(214, -1);
    sidebar.append(&brand());

    let nav = gtk::ListBox::new();
    nav.add_css_class("navigation-sidebar");
    nav.set_selection_mode(gtk::SelectionMode::Single);
    nav.set_vexpand(true);
    sidebar.append(&nav);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .vexpand(true)
        .build();

    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.append(&sidebar);
    split.append(&stack);

    let banner = adw::Banner::new("");
    banner.set_revealed(false);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&split);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&content));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&toasts));
    window.set_content(Some(&toolbar));

    let ui = Rc::new(Window {
        client: Client::new(),
        window: window.clone(),
        toasts,
        banner,
        stack: stack.clone(),
        nav: nav.clone(),
        sections: RefCell::new(Vec::new()),
        rows: RefCell::new(HashMap::new()),
        readings: RefCell::new(HashMap::new()),
        fan_subtitles: RefCell::new(HashMap::new()),
        signature: RefCell::new(String::new()),
        syncing: Cell::new(false),
        sent: RefCell::new(HashMap::new()),
        live: RefCell::new(None),
        live_sensor: RefCell::new(None),
        appearance: RefCell::new(appearance),
    });

    {
        let ui = ui.clone();
        nav.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let index = row.index() as usize;
            if let Some(section) = ui.sections.borrow().get(index) {
                ui.stack.set_visible_child_name(section.id);
            }
        });
    }

    ui.refresh();
    {
        let ui = ui.clone();
        glib::timeout_add_local(REFRESH, move || {
            ui.refresh();
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

/// The application's mark at the top of the sidebar.
fn brand() -> gtk::Box {
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    brand.add_css_class("brand");

    // The installed icon when it is installed, and a stock symbolic when this
    // is a `cargo run` from a checkout -- rather than the broken-image glyph.
    let installed = gtk::gdk::Display::default()
        .map(|d| gtk::IconTheme::for_display(&d).has_icon(APP_ID))
        .unwrap_or(false);
    let icon = gtk::Image::from_icon_name(if installed {
        APP_ID
    } else {
        "weather-windy-symbolic"
    });
    brand.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = gtk::Label::new(Some("Controls"));
    title.add_css_class("app-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some("Backlight and cooling"));
    subtitle.add_css_class("app-subtitle");
    subtitle.set_xalign(0.0);
    text.append(&title);
    text.append(&subtitle);
    brand.append(&text);
    brand
}

impl Window {
    fn toast(&self, message: &str) {
        // Long enough to read a modprobe line off, which is the longest thing
        // that shows up here.
        let toast = adw::Toast::builder().title(message).timeout(6).build();
        self.toasts.add_toast(toast);
    }

    fn refresh(self: &Rc<Self>) {
        // The accent and the glass switch live in Raven Settings, in a file
        // this application only reads. Checking it on the refresh tick is what
        // makes changing the theme there change this window without a restart,
        // and it is one small `read_to_string` every two seconds.
        let appearance = config::appearance();
        if appearance != *self.appearance.borrow() {
            theme::apply(&self.window, &appearance);
            *self.appearance.borrow_mut() = appearance;
        }

        let snapshot = self.client.snapshot();
        let signature = signature(&snapshot);
        if signature != *self.signature.borrow() {
            self.rebuild(&snapshot);
            *self.signature.borrow_mut() = signature;
        }
        self.sync_values(&snapshot);
        self.sync_banner(&snapshot);

        // Keep an open curve editor's marker on the live temperature.
        if let (Some(editor), Some(sensor)) = (
            self.live.borrow().as_ref(),
            self.live_sensor.borrow().as_ref(),
        ) {
            let temp = snapshot
                .readings
                .iter()
                .find(|r| &r.id == sensor)
                .map(|r| r.value);
            editor.set_live_temperature(temp);
        }
    }

    // ---- construction -----------------------------------------------------

    fn rebuild(self: &Rc<Self>, snapshot: &Snapshot) {
        // What page was open, so a rebuild -- a module loading, a GPU waking --
        // does not throw the reader back to the first section.
        let showing = self.stack.visible_child_name().map(|s| s.to_string());

        for section in self.sections.borrow_mut().drain(..) {
            self.stack.remove(&section.page);
        }
        self.nav.remove_all();
        self.rows.borrow_mut().clear();
        self.readings.borrow_mut().clear();
        self.fan_subtitles.borrow_mut().clear();
        self.sent.borrow_mut().clear();

        self.add_keyboard(snapshot);
        self.add_fans(snapshot);
        self.add_sensors(snapshot);
        self.add_about(snapshot);

        // Sidebar rows, in the order the sections were added.
        for section in self.sections.borrow().iter() {
            self.nav.append(&nav_row(section));
        }

        let index = showing
            .and_then(|name| self.sections.borrow().iter().position(|s| s.id == name))
            .unwrap_or(0);
        if let Some(row) = self.nav.row_at_index(index as i32) {
            self.nav.select_row(Some(&row));
        }
    }

    /// Add a section, and return the box its cards go in.
    ///
    /// Sections are added only when there is something to say in them, which is
    /// how a desktop avoids a Keyboard entry and a machine with no sensors
    /// avoids a Sensors one.
    fn section(
        self: &Rc<Self>,
        id: &'static str,
        title: &'static str,
        icon: &'static str,
        subtitle: &str,
    ) -> gtk::Box {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        page.add_css_class("page");

        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("page-title");
        heading.set_xalign(0.0);
        page.append(&heading);
        if !subtitle.is_empty() {
            let sub = gtk::Label::new(Some(subtitle));
            sub.add_css_class("page-subtitle");
            sub.set_xalign(0.0);
            sub.set_wrap(true);
            sub.set_max_width_chars(64);
            page.append(&sub);
        }

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&page)
            .build();
        scroll.add_css_class("page-scroll");
        self.stack.add_named(&scroll, Some(id));

        self.sections.borrow_mut().push(Section {
            id,
            title,
            icon,
            page: scroll.clone().upcast::<gtk::Widget>(),
        });
        page
    }

    fn group(page: &gtk::Box, title: &str, description: Option<&str>) -> adw::PreferencesGroup {
        let mut builder = adw::PreferencesGroup::builder().title(title);
        if let Some(description) = description {
            builder = builder.description(description);
        }
        let group = builder.build();
        page.append(&group);
        group
    }

    fn add_keyboard(self: &Rc<Self>, snapshot: &Snapshot) {
        let knobs = snapshot.knobs_for(Role::KeyboardBacklight);
        if knobs.is_empty() {
            // A desktop has no keyboard light and does not want a page saying
            // so; a laptop whose light was not found does. Both are covered by
            // only adding the section when there is a backlight -- the machine
            // that has one and hides it is diagnosed on the Fans page, which is
            // where the module advice already lives.
            return;
        }
        let page = self.section(
            "keyboard",
            "Keyboard",
            "input-keyboard-symbolic",
            "Found through the kernel LED class, which is how every driver with \
             a keyboard light publishes one.",
        );
        let group = Self::group(&page, "Backlight", None);
        for knob in knobs {
            self.add_knob_row(&group, knob);
        }
    }

    fn add_fans(self: &Rc<Self>, snapshot: &Snapshot) {
        let duties = snapshot.knobs_for(Role::FanDuty);
        let modes = snapshot.knobs_for(Role::FanControlMode);
        let thermal = snapshot.knobs_for(Role::ThermalProfile);
        let nothing = duties.is_empty() && modes.is_empty() && thermal.is_empty();

        // Unlike the keyboard, an absent fan interface is worth a page: it is
        // where the diagnosis goes, and "there is nothing here and this is why"
        // is the most useful thing this application can say on such a machine.
        let page = self.section(
            "fans",
            "Fans",
            "weather-windy-symbolic",
            if nothing {
                "Nothing on this machine exposes a fan the kernel can drive."
            } else {
                "Duty cycles through hwmon, and the coarser modes firmware offers."
            },
        );

        if !duties.is_empty() || !modes.is_empty() {
            let group = Self::group(&page, "Fan control", None);
            for knob in &duties {
                let row = self.add_knob_row(&group, knob);
                // A curve needs a temperature to follow, and a daemon to run it.
                if snapshot.via == Via::Daemon && !snapshot.temperatures().is_empty() {
                    self.attach_curve_button(&row, knob, snapshot);
                }
                self.fan_subtitles
                    .borrow_mut()
                    .insert(knob.id.clone(), row.clone());
            }
            for knob in modes {
                self.add_knob_row(&group, knob);
            }
        }

        if !thermal.is_empty() {
            let group = Self::group(
                &page,
                "Thermal profile",
                Some(
                    "Firmware-managed modes. Coarser than a fan curve, and available on many \
                     laptops that expose nothing finer.",
                ),
            );
            for knob in thermal {
                self.add_knob_row(&group, knob);
            }
        }

        if nothing && !snapshot.diagnosis.is_empty() {
            let group = Self::group(
                &page,
                "What would give this machine fan control",
                Some(
                    "Read from the module list and the kernel's own configuration, so a \
                     driver that was never compiled is not suggested as though it were.",
                ),
            );
            for line in &snapshot.diagnosis {
                let (title, body) = match line.split_once('\n') {
                    Some((title, body)) => (title, body.trim().to_string()),
                    None => (line.as_str(), String::new()),
                };
                let row = adw::ActionRow::builder().title(title).build();
                row.set_title_lines(0);
                if !body.is_empty() {
                    // The command to type, in the font you would type it in.
                    let command = gtk::Label::new(Some(&body));
                    command.add_css_class("mono");
                    command.add_css_class("dim-label");
                    command.set_valign(gtk::Align::Center);
                    command.set_selectable(true);
                    row.add_suffix(&command);
                }
                group.add(&row);
            }
        }
    }

    fn add_sensors(self: &Rc<Self>, snapshot: &Snapshot) {
        if snapshot.readings.is_empty() {
            return;
        }
        let page = self.section(
            "sensors",
            "Sensors",
            "utilities-system-monitor-symbolic",
            "Every fan speed and temperature the kernel publishes for this machine.",
        );
        let group = Self::group(&page, "Live readings", None);
        for reading in &snapshot.readings {
            let row = adw::ActionRow::builder()
                .title(&reading.label)
                .subtitle(reading.unit.format(reading.value))
                .build();
            row.add_css_class("property");
            group.add(&row);
            self.readings.borrow_mut().insert(reading.id.clone(), row);
        }
    }

    fn add_about(self: &Rc<Self>, snapshot: &Snapshot) {
        let page = self.section(
            "about",
            "About",
            "computer-symbolic",
            "What RavenControls found, and how it is writing to it.",
        );

        let group = Self::group(&page, "This machine", None);
        let machine = adw::ActionRow::builder()
            .title(&snapshot.machine)
            .subtitle(match snapshot.via {
                Via::Daemon => "Writing through raven-controlsd".to_string(),
                Via::Sysfs => "Writing straight to sysfs; raven-controlsd is not running".into(),
            })
            .build();
        machine.add_css_class("property");
        group.add(&machine);

        // Which provider found what. On a machine that behaves oddly this is
        // the first thing worth seeing, and it saves a trip to the terminal
        // for `--probe`.
        let providers = Self::group(
            &page,
            "Providers",
            Some("Generic interfaces first; a vendor path is only reached when the generic one found nothing."),
        );
        let mut counts: Vec<(String, usize)> = Vec::new();
        for knob in &snapshot.knobs {
            match counts.iter_mut().find(|(name, _)| name == &knob.provider) {
                Some((_, n)) => *n += 1,
                None => counts.push((knob.provider.clone(), 1)),
            }
        }
        if counts.is_empty() {
            counts.push(("none".into(), 0));
        }
        for (provider, count) in counts {
            let row = adw::ActionRow::builder()
                .title(&provider)
                .subtitle(match count {
                    0 => "found nothing".to_string(),
                    1 => "1 control".to_string(),
                    n => format!("{n} controls"),
                })
                .build();
            row.add_css_class("property");
            providers.add(&row);
        }
    }

    /// One control, rendered from its domain.
    fn add_knob_row(self: &Rc<Self>, group: &adw::PreferencesGroup, knob: &Knob) -> adw::ActionRow {
        let placeholder = adw::ActionRow::new();
        match &knob.domain {
            Domain::Percent { raw_max } => {
                let row = adw::ActionRow::builder().title(&knob.label).build();
                let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
                scale.set_width_request(220);
                scale.set_draw_value(true);
                scale.set_value_pos(gtk::PositionType::Right);
                scale.set_valign(gtk::Align::Center);
                // Snap the slider to the steps the hardware actually has. On a
                // four-step keyboard light a free-running slider produces three
                // positions that all mean the same thing.
                if *raw_max > 0 && *raw_max <= 24 {
                    scale.set_round_digits(0);
                    for step in 0..=*raw_max {
                        scale.add_mark(
                            raw_to_percent(step, *raw_max),
                            gtk::PositionType::Bottom,
                            None,
                        );
                    }
                    scale.set_increments(100.0 / *raw_max as f64, 100.0 / *raw_max as f64);
                }
                scale.set_sensitive(knob.writable);
                row.add_suffix(&scale);
                row.set_activatable_widget(Some(&scale));
                self.explain_if_read_only(&row, knob);
                group.add(&row);

                let this = self.clone();
                let knob_for_handler = knob.clone();
                let raw_max = *raw_max;
                scale.connect_value_changed(move |scale| {
                    if this.syncing.get() {
                        return;
                    }
                    // Quantise before comparing: dragging across a step of a
                    // coarse control must produce one write, not forty.
                    let raw = percent_to_raw(scale.value(), raw_max);
                    let value = Setting::Percent {
                        percent: raw_to_percent(raw, raw_max),
                    };
                    this.send(&knob_for_handler, value);
                });

                self.rows
                    .borrow_mut()
                    .insert(knob.id.clone(), RowWidget::Scale { scale });
                row
            }
            Domain::Switch => {
                let row = adw::SwitchRow::builder()
                    .title(&knob.label)
                    .sensitive(knob.writable)
                    .build();
                group.add(&row);
                let this = self.clone();
                let knob_for_handler = knob.clone();
                row.connect_active_notify(move |row| {
                    if this.syncing.get() {
                        return;
                    }
                    this.send(
                        &knob_for_handler,
                        Setting::Switch {
                            on: row.is_active(),
                        },
                    );
                });
                self.rows
                    .borrow_mut()
                    .insert(knob.id.clone(), RowWidget::Switch(row));
                placeholder
            }
            Domain::Modes { options } | Domain::Steps { labels: options } => {
                let strings: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
                let row = adw::ComboRow::builder()
                    .title(&knob.label)
                    .model(&gtk::StringList::new(&strings))
                    .sensitive(knob.writable)
                    .build();
                self.explain_if_read_only_combo(&row, knob);
                group.add(&row);

                let this = self.clone();
                let knob_for_handler = knob.clone();
                let options_for_handler = options.clone();
                let stepped = matches!(knob.domain, Domain::Steps { .. });
                row.connect_selected_notify(move |row| {
                    if this.syncing.get() {
                        return;
                    }
                    let index = row.selected() as usize;
                    let value = if stepped {
                        Setting::Step { index }
                    } else {
                        let Some(name) = options_for_handler.get(index) else {
                            return;
                        };
                        Setting::Mode { name: name.clone() }
                    };
                    this.send(&knob_for_handler, value);
                });

                self.rows.borrow_mut().insert(
                    knob.id.clone(),
                    RowWidget::Combo {
                        row,
                        options: options.clone(),
                    },
                );
                placeholder
            }
            Domain::Color { channels, raw_max } => {
                let row = adw::ActionRow::builder().title(&knob.label).build();
                match rgb_indices(channels) {
                    Some(indices) => {
                        let button = gtk::ColorDialogButton::builder()
                            .dialog(&gtk::ColorDialog::new())
                            .valign(gtk::Align::Center)
                            .sensitive(knob.writable)
                            .build();
                        row.add_suffix(&button);
                        self.explain_if_read_only(&row, knob);
                        group.add(&row);

                        let this = self.clone();
                        let knob_for_handler = knob.clone();
                        let raw_max = *raw_max;
                        let channel_count = channels.len();
                        button.connect_rgba_notify(move |button| {
                            if this.syncing.get() {
                                return;
                            }
                            let rgba = button.rgba();
                            let mut intensities = vec![0u32; channel_count];
                            let scale = raw_max as f32;
                            intensities[indices.0] = (rgba.red() * scale).round() as u32;
                            intensities[indices.1] = (rgba.green() * scale).round() as u32;
                            intensities[indices.2] = (rgba.blue() * scale).round() as u32;
                            this.send(&knob_for_handler, Setting::Color { intensities });
                        });

                        self.rows.borrow_mut().insert(
                            knob.id.clone(),
                            RowWidget::Color {
                                button,
                                channels: channels.clone(),
                                raw_max,
                            },
                        );
                    }
                    None => {
                        // A keyboard whose channels are not red/green/blue --
                        // white-and-amber backlights exist. One slider each is
                        // less pretty than a colour wheel and is correct for
                        // any channel set the kernel might publish.
                        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
                        let mut scales = Vec::new();
                        for channel in channels {
                            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            line.append(
                                &gtk::Label::builder()
                                    .label(channel)
                                    .width_chars(8)
                                    .xalign(0.0)
                                    .build(),
                            );
                            let scale = gtk::Scale::with_range(
                                gtk::Orientation::Horizontal,
                                0.0,
                                *raw_max as f64,
                                1.0,
                            );
                            scale.set_width_request(160);
                            scale.set_sensitive(knob.writable);
                            line.append(&scale);
                            box_.append(&line);
                            scales.push(scale);
                        }
                        row.add_suffix(&box_);
                        group.add(&row);

                        for (i, scale) in scales.iter().enumerate() {
                            let this = self.clone();
                            let knob_for_handler = knob.clone();
                            let all = scales.clone();
                            let _ = i;
                            scale.connect_value_changed(move |_| {
                                if this.syncing.get() {
                                    return;
                                }
                                let intensities =
                                    all.iter().map(|s| s.value().round() as u32).collect();
                                this.send(&knob_for_handler, Setting::Color { intensities });
                            });
                        }

                        self.rows
                            .borrow_mut()
                            .insert(knob.id.clone(), RowWidget::Channels { scales });
                    }
                }
                row
            }
        }
    }

    fn explain_if_read_only(&self, row: &adw::ActionRow, knob: &Knob) {
        if !knob.writable {
            row.set_subtitle(&read_only_reason(knob));
        }
    }

    fn explain_if_read_only_combo(&self, row: &adw::ComboRow, knob: &Knob) {
        if !knob.writable {
            row.set_subtitle(&read_only_reason(knob));
        }
    }

    fn attach_curve_button(
        self: &Rc<Self>,
        row: &adw::ActionRow,
        knob: &Knob,
        snapshot: &Snapshot,
    ) {
        let button = gtk::Button::builder()
            .label("Curve…")
            .valign(gtk::Align::Center)
            .sensitive(knob.writable)
            .build();
        row.add_suffix(&button);

        let this = self.clone();
        let knob = knob.clone();
        let sensors: Vec<(String, String)> = snapshot
            .temperatures()
            .iter()
            .map(|r| (r.id.clone(), r.label.clone()))
            .collect();
        let existing = snapshot.driving(&knob.id).cloned();
        button.connect_clicked(move |button| {
            let (curve, selected) = match &existing {
                Some(driving) => (driving.curve.clone(), Some(driving.sensor.clone())),
                None => (
                    raven_hw::curve::Curve::default(),
                    // Default to the hottest sensor, which on a laptop is the
                    // CPU package and on a desktop is whatever is working.
                    sensors.first().map(|(id, _)| id.clone()),
                ),
            };
            *this.live_sensor.borrow_mut() = selected.clone();

            let apply = {
                let this = this.clone();
                let knob = knob.clone();
                move |curve: raven_hw::curve::Curve, sensor: String| {
                    match this.client.drive(&knob.id, &sensor, curve) {
                        Ok(()) => this.toast(&format!("{} is following the curve", knob.label)),
                        Err(e) => this.toast(&e.to_string()),
                    }
                    *this.live.borrow_mut() = None;
                    this.signature.borrow_mut().clear();
                }
            };
            let stop = {
                let this = this.clone();
                let knob = knob.clone();
                move || {
                    match this.client.release(&knob.id) {
                        Ok(()) => this.toast(&format!("{} handed back to firmware", knob.label)),
                        Err(e) => this.toast(&e.to_string()),
                    }
                    *this.live.borrow_mut() = None;
                    this.signature.borrow_mut().clear();
                }
            };

            let editor = curve::present(
                button,
                &knob.label,
                curve,
                sensors.clone(),
                selected,
                apply,
                stop,
            );
            *this.live.borrow_mut() = Some(editor);
        });
    }

    // ---- refresh ----------------------------------------------------------

    fn sync_values(self: &Rc<Self>, snapshot: &Snapshot) {
        self.syncing.set(true);

        for knob in &snapshot.knobs {
            let rows = self.rows.borrow();
            let Some(widget) = rows.get(&knob.id) else {
                continue;
            };
            match (widget, &knob.value) {
                (RowWidget::Scale { scale }, Setting::Percent { percent }) => {
                    if (scale.value() - percent).abs() > 0.5 {
                        scale.set_value(*percent);
                    }
                }
                (RowWidget::Switch(row), Setting::Switch { on }) => {
                    if row.is_active() != *on {
                        row.set_active(*on);
                    }
                }
                (RowWidget::Combo { row, options }, Setting::Mode { name }) => {
                    if let Some(i) = options.iter().position(|o| o == name) {
                        if row.selected() != i as u32 {
                            row.set_selected(i as u32);
                        }
                    }
                }
                (RowWidget::Combo { row, .. }, Setting::Step { index }) => {
                    if row.selected() != *index as u32 {
                        row.set_selected(*index as u32);
                    }
                }
                (
                    RowWidget::Color {
                        button,
                        channels,
                        raw_max,
                    },
                    Setting::Color { intensities },
                ) => {
                    if let Some((r, g, b)) = rgb_indices(channels) {
                        let scale = *raw_max as f32;
                        let get = |i: usize| {
                            intensities.get(i).copied().unwrap_or(0) as f32 / scale.max(1.0)
                        };
                        let rgba = gtk::gdk::RGBA::new(get(r), get(g), get(b), 1.0);
                        if button.rgba() != rgba {
                            button.set_rgba(&rgba);
                        }
                    }
                }
                (RowWidget::Channels { scales }, Setting::Color { intensities }) => {
                    for (scale, value) in scales.iter().zip(intensities) {
                        if (scale.value() - *value as f64).abs() > 0.5 {
                            scale.set_value(*value as f64);
                        }
                    }
                }
                _ => {}
            }
        }

        for reading in &snapshot.readings {
            if let Some(row) = self.readings.borrow().get(&reading.id) {
                let text = match reading.critical {
                    Some(critical) if reading.value >= critical => {
                        format!(
                            "{} — at the driver's critical temperature",
                            reading.unit.format(reading.value)
                        )
                    }
                    _ => reading.unit.format(reading.value),
                };
                if row.subtitle().map(|s| s.to_string()).as_deref() != Some(text.as_str()) {
                    row.set_subtitle(&text);
                }
            }
        }

        // What each driven fan is doing, in words.
        for (knob_id, row) in self.fan_subtitles.borrow().iter() {
            let text = match snapshot.driving(knob_id) {
                Some(d) => {
                    let why = match d.reason.as_str() {
                        "critical" => " — over the critical temperature, at full speed",
                        "held" => " — holding, waiting out the hysteresis band",
                        _ => "",
                    };
                    format!(
                        "Following a curve on {} at {:.0} °C{why}",
                        short_sensor(&d.sensor),
                        d.temp_c
                    )
                }
                None => match snapshot.knobs.iter().find(|k| &k.id == knob_id) {
                    Some(k) if !k.writable => read_only_reason(k),
                    _ if snapshot.via == Via::Sysfs => {
                        "Manual control needs raven-controlsd".to_string()
                    }
                    _ => String::new(),
                },
            };
            if row.subtitle().map(|s| s.to_string()).unwrap_or_default() != text {
                row.set_subtitle(&text);
            }
        }

        self.syncing.set(false);
    }

    fn sync_banner(&self, snapshot: &Snapshot) {
        // The diagnosis is a card on the Fans page, not a banner: it is several
        // lines long, it names commands worth selecting, and a banner is the
        // wrong shape for both. The banner carries the one thing that is true
        // of the whole window regardless of which page is open.
        let needs_daemon =
            snapshot.via == Via::Sysfs && snapshot.knobs.iter().any(|k| k.role == Role::FanDuty);
        if needs_daemon {
            self.banner.set_title(
                "Fan curves and manual fan speeds need raven-controlsd, which hands the \
                 fans back to firmware if this window stops running. Start it with: \
                 sudo raven-controlsd",
            );
            self.banner.set_revealed(true);
            return;
        }
        self.banner.set_revealed(false);
    }

    /// Write a value, unless it is the one already there.
    fn send(self: &Rc<Self>, knob: &Knob, value: Setting) {
        if self.sent.borrow().get(&knob.id) == Some(&value) {
            return;
        }
        match self.client.set(knob, &value) {
            Ok(()) => {
                self.sent.borrow_mut().insert(knob.id.clone(), value);
            }
            Err(e) => {
                self.toast(&e.to_string());
                // Put the widget back where the hardware actually is, rather
                // than leaving a slider showing a value that was refused.
                self.signature.borrow_mut().clear();
            }
        }
    }
}

/// One sidebar entry.
fn nav_row(section: &Section) -> gtk::ListBoxRow {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    box_.append(&gtk::Image::from_icon_name(section.icon));
    let label = gtk::Label::new(Some(section.title));
    label.set_xalign(0.0);
    box_.append(&label);
    gtk::ListBoxRow::builder().child(&box_).build()
}

/// Which entries of a multicolour LED's `multi_index` are red, green and blue.
///
/// The kernel does not promise the order, and does not promise those three are
/// what a device has: `multi_index` is whatever the driver registered.
fn rgb_indices(channels: &[String]) -> Option<(usize, usize, usize)> {
    let find = |name: &str| channels.iter().position(|c| c.eq_ignore_ascii_case(name));
    Some((find("red")?, find("green")?, find("blue")?))
}

fn read_only_reason(knob: &Knob) -> String {
    format!(
        "Read-only for this account: {} is owned by root. Run `imlazy install-udev` to let \
         the video group set it, or start raven-controlsd.",
        knob.origin
    )
}

/// `hwmon/nct6798/temp1` reads better as `temp1` in a sentence.
fn short_sensor(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

/// What the window is built from. When this changes, the rows are rebuilt;
/// while it does not, values are updated in place and a slider under a finger
/// stays put.
fn signature(snapshot: &Snapshot) -> String {
    let mut parts: Vec<String> = snapshot
        .knobs
        .iter()
        .map(|k| format!("{}:{}:{}", k.id, k.writable, domain_tag(&k.domain)))
        .collect();
    parts.extend(snapshot.readings.iter().map(|r| r.id.clone()));
    parts.push(format!("via={:?}", snapshot.via));
    parts.push(format!("driving={}", snapshot.driving.len()));
    parts.push(format!("diagnosis={}", snapshot.diagnosis.len()));
    parts.join("|")
}

/// Domains are part of the signature because a driver can change one: an
/// `nct6798` moved out of Smart Fan IV loses a mode from its menu, and a
/// dropdown left listing the old set would write the wrong value.
fn domain_tag(domain: &Domain) -> String {
    match domain {
        Domain::Percent { raw_max } => format!("percent{raw_max}"),
        Domain::Switch => "switch".into(),
        Domain::Modes { options } => format!("modes[{}]", options.join(",")),
        Domain::Steps { labels } => format!("steps{}", labels.len()),
        Domain::Color { channels, raw_max } => {
            format!("color[{}]{raw_max}", channels.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_channels_are_found_whatever_order_the_driver_registered_them() {
        let channels = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            rgb_indices(&channels(&["red", "green", "blue"])),
            Some((0, 1, 2))
        );
        assert_eq!(
            rgb_indices(&channels(&["blue", "red", "green"])),
            Some((1, 2, 0))
        );
        assert_eq!(
            rgb_indices(&channels(&["RED", "GREEN", "BLUE"])),
            Some((0, 1, 2))
        );
    }

    #[test]
    fn a_keyboard_that_is_not_rgb_falls_back_to_per_channel_sliders() {
        let channels = vec!["white".to_string(), "amber".to_string()];
        assert_eq!(rgb_indices(&channels), None);
    }

    #[test]
    fn a_changed_mode_list_rebuilds_the_row_rather_than_reusing_a_stale_menu() {
        let a = Domain::Modes {
            options: vec!["Automatic".into(), "Manual".into()],
        };
        let b = Domain::Modes {
            options: vec!["Automatic (driver mode 5)".into(), "Manual".into()],
        };
        assert_ne!(domain_tag(&a), domain_tag(&b));
    }

    #[test]
    fn a_sensor_id_shortens_to_something_readable_in_a_sentence() {
        assert_eq!(short_sensor("hwmon/nct6798/temp1"), "temp1");
        assert_eq!(short_sensor("temp1"), "temp1");
    }
}
