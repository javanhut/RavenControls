//! `raven-controlsd` -- the half of RavenControls that is allowed to write.
//!
//! It exists for one reason: a fan taken off firmware control has to be given
//! back. Writing `pwmN_enable=1` from a settings window means the fan is pinned
//! wherever the window left it the moment the window dies -- and windows die,
//! get killed, get upgraded, and get closed by people who assume closing a
//! window undoes what it did. A daemon can outlive the window, run the curve,
//! and hand every channel back on the way out.
//!
//! It is deliberately small. It does not decide policy, it does not talk to a
//! display, and it holds no vendor knowledge -- all of that is `raven-hw`.
//!
//! Run as root:
//!
//! ```text
//! sudo raven-controlsd
//! ```
//!
//! and it listens on /run/raven-controls/ctl for the `video` group.

use raven_controlsd::{protocol, state, supervisor};

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use raven_hw::ipc::{GROUP, SOCKET};
use raven_hw::Root;

use crate::supervisor::Supervisor;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "raven-controlsd -- fan and backlight daemon for RavenControls\n\n\
             Usage: raven-controlsd [--socket PATH] [--state PATH]\n\n\
             Runs as root, listens on {SOCKET} for the {GROUP} group, and hands every\n\
             fan back to firmware when it exits."
        );
        return Ok(());
    }
    let socket_path = flag(&args, "--socket").unwrap_or_else(|| SOCKET.into());
    let state_path = flag(&args, "--state").unwrap_or_else(|| state::DEFAULT_PATH.into());

    if uid() != 0 {
        anyhow::bail!(
            "raven-controlsd writes pwmN_enable, which is root-only. Run it as root \
             (sudo raven-controlsd), or -- for the keyboard light alone -- install the \
             udev rule and use raven-controls without the daemon."
        );
    }

    let root = Root::system();
    tracing::info!("{}", raven_hw::machine(&root));

    let supervisor = Arc::new(Mutex::new(Supervisor::new(
        root.clone(),
        PathBuf::from(&state_path),
    )));
    supervisor.lock().unwrap().restore_saved();

    spawn_signal_handler(supervisor.clone(), root.clone(), socket_path.clone());
    spawn_control_loop(supervisor.clone());
    spawn_watchdog(supervisor.clone(), root.clone());

    let listener = listen(&socket_path)?;
    tracing::info!("listening on {socket_path} for group {GROUP}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let supervisor = supervisor.clone();
                // A thread per connection. There will be one or two of them --
                // a window and perhaps a shell -- and a thread is far less code
                // than a poll loop for that.
                std::thread::spawn(move || {
                    if let Err(e) = protocol::serve(stream, supervisor) {
                        tracing::debug!("client gone: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept: {e}"),
        }
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

fn uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:"))?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(u32::MAX)
}

/// The numeric gid of a group, from `/etc/group`.
///
/// Parsed rather than looked up through NSS because that would mean linking
/// libc's resolver into a daemon whose entire job is writing four bytes to
/// sysfs, and because the groups that matter here are local by definition.
fn gid_of(group: &str) -> Option<u32> {
    std::fs::read_to_string("/etc/group")
        .ok()?
        .lines()
        .find_map(|line| {
            let mut f = line.split(':');
            (f.next()? == group).then(|| f.nth(1)?.parse().ok())?
        })
}

fn listen(path: &str) -> anyhow::Result<UnixListener> {
    let path = PathBuf::from(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    }
    // A socket left by a previous run would make bind fail. Removing it is only
    // safe because we are the only thing that creates it and we are root.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;

    // 0660 root:video. Not 0666: anything that can reach this socket can pin a
    // fan at zero.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    match gid_of(GROUP) {
        Some(gid) => std::os::unix::fs::chown(&path, Some(0), Some(gid))?,
        None => tracing::warn!(
            "there is no {GROUP} group on this system, so only root can use {}",
            path.display()
        ),
    }
    Ok(listener)
}

fn spawn_control_loop(supervisor: Arc<Mutex<Supervisor>>) {
    std::thread::spawn(move || {
        let mut last = Instant::now();
        loop {
            {
                let mut s = supervisor.lock().unwrap();
                s.tick();
            }
            let next = supervisor::next_deadline(last);
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            last = next;
        }
    });
}

/// Watches the loop that watches the fans.
///
/// If a tick blocks -- a sysfs read into a wedged driver is the realistic way
/// this happens -- the fans stay at whatever the last tick set while the
/// machine heats up, and nothing else would notice. This notices, and hands
/// every channel back to firmware without needing the lock the stuck thread is
/// holding.
fn spawn_watchdog(supervisor: Arc<Mutex<Supervisor>>, root: Root) {
    let heartbeat = supervisor.lock().unwrap().heartbeat();
    std::thread::spawn(move || {
        let mut tripped = false;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let driving = supervisor
                .try_lock()
                .map(|s| s.is_driving_anything())
                // Not being able to take the lock is itself a reason to keep
                // watching, so assume the worst.
                .unwrap_or(true);
            if !driving {
                tripped = false;
                continue;
            }
            let age = now_secs().saturating_sub(heartbeat.load(Ordering::Relaxed));
            if age > supervisor::WATCHDOG.as_secs() {
                if !tripped {
                    tracing::error!(
                        "the fan control loop has not run for {age}s. Handing every fan \
                         back to firmware."
                    );
                    emergency_release(&root);
                    tripped = true;
                }
            } else {
                tripped = false;
            }
        }
    });
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Put every hwmon PWM channel back on firmware control, without consulting any
/// of our own bookkeeping.
///
/// Used from the watchdog and from the signal handler when the supervisor's
/// lock cannot be taken -- exactly the cases where our bookkeeping is the thing
/// that might be broken. It will also revert a channel someone set to manual
/// outside RavenControls, which is a real cost and the right trade: on this
/// path the alternative is a fan stuck at a duty nobody is maintaining.
fn emergency_release(root: &Root) {
    for chip in raven_hw::fan::hwmon::chips(root) {
        for (name, path) in root.list(&chip.dir) {
            if !name.starts_with("pwm") || !name.ends_with("_enable") {
                continue;
            }
            if root.read(&path).as_deref() == Some("1") {
                match root.write(&path, "2") {
                    Ok(()) => tracing::warn!("{path} handed back to firmware"),
                    Err(e) => tracing::error!("{path} could not be handed back: {e}"),
                }
            }
        }
    }
}

/// SIGTERM and SIGINT, turned into an ordinary exit that releases the fans.
///
/// `Guard`'s `Drop` covers panics and returns; it does not cover a signal,
/// which is what `raven-rc stop`, a reboot and Ctrl-C all send. SIGKILL is
/// still uncoverable by anything in this process -- the watchdog in whatever
/// starts us next is the answer there, and `restore_saved` re-applies curves
/// onto channels firmware has meanwhile taken back.
fn spawn_signal_handler(supervisor: Arc<Mutex<Supervisor>>, root: Root, socket: String) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    let mut signals = match Signals::new([SIGTERM, SIGINT, SIGHUP]) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("could not install signal handlers: {e}");
            return;
        }
    };
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            tracing::info!("signal {signal}; releasing every fan");
            // A short wait for the control loop to finish its tick, then the
            // path that needs no lock at all.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Ok(mut s) = supervisor.try_lock() {
                    s.release_all();
                    break;
                }
                if Instant::now() >= deadline {
                    tracing::warn!("the control loop is busy; releasing without it");
                    emergency_release(&root);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = std::fs::remove_file(&socket);
            std::process::exit(0);
        }
    });
}
