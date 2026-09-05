//! The fan curve editor: a graph you drag.
//!
//! A curve is five numbers in pairs, and every text-field version of this is
//! miserable to use, because the thing being edited is a shape. So it is a
//! drawing area with draggable points, drawn from the same `raven_hw::curve`
//! the daemon runs -- the preview line and the fan follow the same code.
//!
//! The live temperature is drawn on it as a moving marker, which is the detail
//! that makes the editor legible: you can see where the machine actually sits
//! before deciding where the knee belongs.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;

use raven_hw::curve::{Curve, Point};

/// The temperature range drawn. Below 20 °C nothing runs, and above 105 °C
/// every machine has already thermally shut down.
const T_MIN: f64 = 20.0;
const T_MAX: f64 = 105.0;

/// How near, in pixels, a click has to be to grab a point.
const GRAB: f64 = 16.0;

pub struct CurveEditor {
    pub widget: gtk::DrawingArea,
    points: Rc<RefCell<Vec<Point>>>,
    live: Rc<RefCell<Option<f64>>>,
}

fn clamp_point(p: Point) -> Point {
    Point {
        temp_c: p.temp_c.clamp(T_MIN, T_MAX),
        percent: p.percent.clamp(0.0, 100.0),
    }
}

impl CurveEditor {
    pub fn new(curve: &Curve) -> Rc<Self> {
        let widget = gtk::DrawingArea::builder()
            .height_request(240)
            .hexpand(true)
            .build();
        let editor = Rc::new(Self {
            widget: widget.clone(),
            points: Rc::new(RefCell::new(curve.points.clone())),
            live: Rc::new(RefCell::new(None)),
        });

        {
            let points = editor.points.clone();
            let live = editor.live.clone();
            widget.set_draw_func(move |area, cr, width, height| {
                draw(
                    area,
                    cr,
                    width as f64,
                    height as f64,
                    &points.borrow(),
                    *live.borrow(),
                );
            });
        }

        editor.install_gestures();
        editor
    }

    pub fn points(&self) -> Vec<Point> {
        self.points.borrow().clone()
    }

    /// Where the machine is right now, drawn as a marker on the graph.
    pub fn set_live_temperature(&self, temp_c: Option<f64>) {
        *self.live.borrow_mut() = temp_c;
        self.widget.queue_draw();
    }

    fn install_gestures(self: &Rc<Self>) {
        // Drag an existing point.
        let drag = gtk::GestureDrag::new();
        let dragging: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        {
            let this = self.clone();
            let dragging = dragging.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let (w, h) = (this.widget.width() as f64, this.widget.height() as f64);
                *dragging.borrow_mut() = nearest(&this.points.borrow(), w, h, x, y);
            });
        }
        {
            let this = self.clone();
            let dragging = dragging.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(index) = *dragging.borrow() else {
                    return;
                };
                let Some((sx, sy)) = g.start_point() else {
                    return;
                };
                let (w, h) = (this.widget.width() as f64, this.widget.height() as f64);
                let mut points = this.points.borrow_mut();
                points[index] = clamp_point(from_pixels(w, h, sx + dx, sy + dy));
                // Keep the list sorted by temperature as it is dragged, and
                // follow the point being moved so the drag does not jump to a
                // neighbour when two cross.
                let moved = points[index];
                points.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));
                let new_index = points
                    .iter()
                    .position(|p| p.temp_c == moved.temp_c && p.percent == moved.percent)
                    .unwrap_or(index);
                drop(points);
                *dragging.borrow_mut() = Some(new_index);
                this.widget.queue_draw();
            });
        }
        {
            let dragging = dragging.clone();
            drag.connect_drag_end(move |_, _, _| {
                *dragging.borrow_mut() = None;
            });
        }
        self.widget.add_controller(drag);

        // A click on empty space adds a point; a right-click or a click on an
        // existing one removes it.
        let click = gtk::GestureClick::new();
        click.set_button(0);
        {
            let this = self.clone();
            click.connect_released(move |g, n, x, y| {
                if n != 1 {
                    return;
                }
                let (w, h) = (this.widget.width() as f64, this.widget.height() as f64);
                let hit = nearest(&this.points.borrow(), w, h, x, y);
                let secondary = g.current_button() == gdk::BUTTON_SECONDARY;
                let mut points = this.points.borrow_mut();
                match (hit, secondary) {
                    (Some(index), true) => {
                        // A curve with no points has nothing to say, so the
                        // last one cannot be removed.
                        if points.len() > 1 {
                            points.remove(index);
                        }
                    }
                    (None, false) => {
                        points.push(clamp_point(from_pixels(w, h, x, y)));
                        points.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));
                    }
                    _ => {}
                }
                drop(points);
                this.widget.queue_draw();
            });
        }
        self.widget.add_controller(click);
    }
}

// ---- geometry -------------------------------------------------------------

/// The plot area inside the widget, leaving room for the axis labels.
fn inset(w: f64, h: f64) -> (f64, f64, f64, f64) {
    let (left, right, top, bottom) = (38.0, 12.0, 12.0, 24.0);
    (
        left,
        top,
        (w - left - right).max(1.0),
        (h - top - bottom).max(1.0),
    )
}

fn to_pixels(w: f64, h: f64, p: Point) -> (f64, f64) {
    let (x0, y0, pw, ph) = inset(w, h);
    (
        x0 + (p.temp_c - T_MIN) / (T_MAX - T_MIN) * pw,
        y0 + (1.0 - p.percent / 100.0) * ph,
    )
}

fn from_pixels(w: f64, h: f64, x: f64, y: f64) -> Point {
    let (x0, y0, pw, ph) = inset(w, h);
    Point {
        temp_c: T_MIN + ((x - x0) / pw) * (T_MAX - T_MIN),
        percent: (1.0 - (y - y0) / ph) * 100.0,
    }
}

fn nearest(points: &[Point], w: f64, h: f64, x: f64, y: f64) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (px, py) = to_pixels(w, h, *p);
            (i, ((px - x).powi(2) + (py - y).powi(2)).sqrt())
        })
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

// ---- drawing --------------------------------------------------------------

fn draw(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    w: f64,
    h: f64,
    points: &[Point],
    live: Option<f64>,
) {
    // Colours come from the theme rather than being hard-coded, so the graph
    // is legible in both light and dark without a second palette.
    let fg = area.color();
    // The platform accent, without the deprecated style context: a colour
    // fetched from the widget tree via a CSS provider would be the "correct"
    // route and is a great deal of machinery for one line. libadwaita's own
    // default blue is what the platform draws with when nothing overrides it.
    let accent = gdk::RGBA::new(0.21, 0.52, 0.89, 1.0);

    let (x0, y0, pw, ph) = inset(w, h);

    // Grid: every 20 degrees, every 25%.
    cr.set_line_width(1.0);
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
    let mut t = 20.0;
    while t <= T_MAX {
        let (x, _) = to_pixels(
            w,
            h,
            Point {
                temp_c: t,
                percent: 0.0,
            },
        );
        cr.move_to(x.floor() + 0.5, y0);
        cr.line_to(x.floor() + 0.5, y0 + ph);
        t += 20.0;
    }
    for p in [0.0, 25.0, 50.0, 75.0, 100.0] {
        let (_, y) = to_pixels(
            w,
            h,
            Point {
                temp_c: T_MIN,
                percent: p,
            },
        );
        cr.move_to(x0, y.floor() + 0.5);
        cr.line_to(x0 + pw, y.floor() + 0.5);
    }
    let _ = cr.stroke();

    // Axis labels.
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.65);
    cr.set_font_size(11.0);
    for p in [0.0, 50.0, 100.0] {
        let (_, y) = to_pixels(
            w,
            h,
            Point {
                temp_c: T_MIN,
                percent: p,
            },
        );
        cr.move_to(4.0, y + 4.0);
        let _ = cr.show_text(&format!("{p:.0}%"));
    }
    let mut t = 20.0;
    while t <= T_MAX {
        let (x, _) = to_pixels(
            w,
            h,
            Point {
                temp_c: t,
                percent: 0.0,
            },
        );
        cr.move_to(x - 10.0, y0 + ph + 16.0);
        let _ = cr.show_text(&format!("{t:.0}°"));
        t += 20.0;
    }

    if points.is_empty() {
        return;
    }

    // The curve itself, sampled through the same interpolation the daemon
    // runs -- including the flat extensions past the outermost points, which
    // is exactly the behaviour worth seeing before committing to a shape.
    let curve = Curve {
        points: points.to_vec(),
        ..Curve::default()
    };
    cr.set_source_rgba(
        accent.red() as f64,
        accent.green() as f64,
        accent.blue() as f64,
        1.0,
    );
    cr.set_line_width(2.0);
    let steps = pw.max(2.0) as usize;
    for i in 0..=steps {
        let temp = T_MIN + (T_MAX - T_MIN) * (i as f64 / steps as f64);
        let (x, y) = to_pixels(
            w,
            h,
            Point {
                temp_c: temp,
                percent: curve.apply_floor(curve.raw_duty_at(temp)),
            },
        );
        if i == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();

    // The handles.
    for p in points {
        let (x, y) = to_pixels(w, h, *p);
        cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }

    // Where the machine is now.
    if let Some(temp) = live {
        let (x, _) = to_pixels(
            w,
            h,
            Point {
                temp_c: temp,
                percent: 0.0,
            },
        );
        if x >= x0 && x <= x0 + pw {
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.55);
            cr.set_line_width(1.5);
            cr.set_dash(&[4.0, 3.0], 0.0);
            cr.move_to(x.floor() + 0.5, y0);
            cr.line_to(x.floor() + 0.5, y0 + ph);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);

            let duty = curve.apply_floor(curve.raw_duty_at(temp));
            let (_, y) = to_pixels(
                w,
                h,
                Point {
                    temp_c: temp,
                    percent: duty,
                },
            );
            cr.arc(x, y, 4.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
}

// ---- the dialog -----------------------------------------------------------

/// Build the editor dialog for one fan.
///
/// `on_apply` receives the curve and the chosen sensor; `on_stop` means hand
/// this fan back to firmware.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    fan_label: &str,
    curve: Curve,
    sensors: Vec<(String, String)>,
    selected_sensor: Option<String>,
    on_apply: impl Fn(Curve, String) + 'static,
    on_stop: impl Fn() + 'static,
) -> Rc<CurveEditor> {
    let editor = CurveEditor::new(&curve);

    let dialog = adw::Dialog::builder()
        .title(format!("Fan curve — {fan_label}"))
        .content_width(560)
        .content_height(660)
        .build();

    let page = adw::PreferencesPage::new();

    let graph_group = adw::PreferencesGroup::builder()
        .title("Curve")
        .description(
            "Drag a point to move it. Click the graph to add one, right-click a point to \
             remove it. The dashed line is the current temperature.",
        )
        .build();
    let frame = gtk::Frame::builder().child(&editor.widget).build();
    graph_group.add(&frame);
    page.add(&graph_group);

    let settings = adw::PreferencesGroup::builder().title("Settings").build();

    let sensor_names: Vec<&str> = sensors.iter().map(|(_, label)| label.as_str()).collect();
    let sensor_row = adw::ComboRow::builder()
        .title("Sensor")
        .subtitle("The temperature this fan follows")
        .model(&gtk::StringList::new(&sensor_names))
        .build();
    if let Some(selected) = &selected_sensor {
        if let Some(i) = sensors.iter().position(|(id, _)| id == selected) {
            sensor_row.set_selected(i as u32);
        }
    }
    settings.add(&sensor_row);

    let hysteresis = adw::SpinRow::with_range(0.0, 15.0, 0.5);
    hysteresis.set_title("Hysteresis");
    hysteresis.set_subtitle("How far the temperature must fall before the fan slows down");
    hysteresis.set_value(curve.hysteresis_c);
    settings.add(&hysteresis);

    let floor = adw::SpinRow::with_range(0.0, 100.0, 1.0);
    floor.set_title("Minimum duty");
    floor.set_subtitle("The lowest speed this fan will actually turn at; below it, it stops");
    floor.set_value(curve.min_percent);
    settings.add(&floor);

    page.add(&settings);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();
    let stop = gtk::Button::builder()
        .label("Hand back to firmware")
        .build();
    let apply = gtk::Button::builder().label("Apply").build();
    apply.add_css_class("suggested-action");
    buttons.append(&stop);
    buttons.append(&apply);
    let button_group = adw::PreferencesGroup::new();
    button_group.add(&buttons);
    page.add(&button_group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    {
        let editor = editor.clone();
        let dialog = dialog.clone();
        let sensors = sensors.clone();
        apply.connect_clicked(move |_| {
            let points = editor.points();
            let sensor = sensors
                .get(sensor_row.selected() as usize)
                .map(|(id, _)| id.clone());
            let (Some(sensor), Ok(mut curve)) = (sensor, Curve::new(points)) else {
                return;
            };
            curve.hysteresis_c = hysteresis.value();
            curve.min_percent = floor.value();
            on_apply(curve, sensor);
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        stop.connect_clicked(move |_| {
            on_stop();
            dialog.close();
        });
    }

    dialog.present(Some(parent));
    editor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_survives_a_round_trip_through_pixel_space() {
        let (w, h) = (600.0, 300.0);
        for p in [
            Point {
                temp_c: 40.0,
                percent: 0.0,
            },
            Point {
                temp_c: 70.0,
                percent: 55.0,
            },
            Point {
                temp_c: T_MAX,
                percent: 100.0,
            },
        ] {
            let (x, y) = to_pixels(w, h, p);
            let back = from_pixels(w, h, x, y);
            assert!((back.temp_c - p.temp_c).abs() < 0.01, "{back:?} vs {p:?}");
            assert!((back.percent - p.percent).abs() < 0.01, "{back:?} vs {p:?}");
        }
    }

    #[test]
    fn dragging_off_the_edge_of_the_graph_clamps_rather_than_producing_nonsense() {
        let p = clamp_point(Point {
            temp_c: -300.0,
            percent: 480.0,
        });
        assert_eq!(p.temp_c, T_MIN);
        assert_eq!(p.percent, 100.0);
    }

    #[test]
    fn a_click_grabs_the_nearest_point_only_when_it_is_close_enough() {
        let (w, h) = (600.0, 300.0);
        let points = vec![
            Point {
                temp_c: 40.0,
                percent: 0.0,
            },
            Point {
                temp_c: 80.0,
                percent: 100.0,
            },
        ];
        let (x, y) = to_pixels(w, h, points[1]);
        assert_eq!(nearest(&points, w, h, x + 2.0, y + 2.0), Some(1));
        // Far from both: a click here adds a point instead of moving one.
        assert_eq!(nearest(&points, w, h, x - 200.0, y + 100.0), None);
    }
}
