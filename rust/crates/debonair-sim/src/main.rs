//! A simulated adapter for developing and testing the app without hardware. It runs the firmware's
//! application logic with a simulated Bluetooth Target and speaks the serial API on standard input
//! and output. Reports the Target receives are written to the file named by `DEBONAIR_SIM_LOG`.
//!
//! Settings are saved as records, as the adapter saves them, in the directory named by
//! `DEBONAIR_SIM_STATE`, or else in a new directory under the system's temporary directory for
//! this process alone, removed when the process exits. A restart ends the process; the app starts
//! it again with the same `DEBONAIR_SIM_STATE`, so settings outlast it.
use debonair_core::{
    ErrorCode,
    app::{App, Build},
    hid::{Channel, Encoded},
    radio::{Config, Event, Radio, Security, Target},
    store::{self, Error, Store},
};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

/// Records as files under one directory, at their store paths.
struct Files(PathBuf);
impl Files {
    fn path(&self, name: &str) -> Result<PathBuf, Error> {
        let relative = name.strip_prefix('/').ok_or(Error::Bounds)?;
        if !relative.is_empty() && relative.split('/').any(|p| matches!(p, "" | "." | "..")) {
            return Err(Error::Bounds);
        }
        Ok(self.0.join(relative))
    }
}
impl Store for Files {
    fn load(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        match fs::read(self.path(name)?) {
            Ok(data) if data.is_empty() => Ok(None),
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(Error::Io),
        }
    }
    fn save(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let path = self.path(name)?;
        if fs::read(&path).is_ok_and(|old| old == bytes) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| Error::Io)?;
        }
        fs::write(path, bytes).map_err(|_| Error::Io)
    }
    fn remove(&mut self, name: &str) -> Result<(), Error> {
        let path = self.path(name)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Io),
        }
        // A record directory, named by an ID, goes with its record.
        if let Some(parent) = path.parent()
            && parent
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(store::parse_id)
                .is_some()
        {
            let _ = fs::remove_dir(parent);
        }
        Ok(())
    }
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
        match fs::read_dir(self.path(directory)?) {
            Ok(entries) => Ok(entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(Error::Io),
        }
    }
}

const TARGET: Target = Target {
    address: [0x02, 0x00, 0x00, 0x12, 0x34, 0x56],
    random: false,
};

/// A Target that pairs a moment after pairing starts and enables smooth scrolling.
struct Simulated {
    now: u64,
    scheduled: Vec<(u64, Event)>,
    events: VecDeque<Event>,
    security: Security,
    pairing: bool,
    connected: bool,
    secured: bool,
    targets: Vec<Target>,
    /// Caps Lock is held in the last keyboard report.
    caps: bool,
    /// The Target's Caps Lock state.
    caps_lock: bool,
    log: Option<fs::File>,
}
impl Simulated {
    fn after(&mut self, ms: u64, event: Event) {
        self.scheduled.push((self.now + ms, event));
    }
    fn tick(&mut self, now: u64) {
        self.now = now;
        let (due, later): (Vec<_>, Vec<_>) =
            self.scheduled.drain(..).partition(|(at, _)| *at <= now);
        self.scheduled = later;
        for (_, event) in due {
            match &event {
                Event::Connected(_) => self.connected = true,
                Event::Secured => self.secured = true,
                Event::Disconnected => {
                    self.connected = false;
                    self.secured = false;
                }
                Event::PairingComplete { success: true } if !self.targets.contains(&TARGET) => {
                    self.targets.push(TARGET);
                }
                _ => {}
            }
            self.events.push_back(event);
        }
    }
    fn secure(&mut self, ms: u64) {
        self.after(ms, Event::Secured);
        self.after(ms, Event::PairingComplete { success: true });
        // Windows and Linux enable the wheel and pan resolution multipliers.
        self.after(
            ms + 50,
            Event::Feature {
                report_id: 2,
                data: heapless::Vec::from_slice(&[0x05]).unwrap(),
            },
        );
    }
    fn log(&mut self, line: &str) {
        if let Some(file) = &mut self.log {
            let _ = writeln!(file, "{line}");
        }
    }
}
impl Radio for Simulated {
    fn start(&mut self, config: &Config<'_>) -> Result<(), ErrorCode> {
        self.security = config.security;
        self.log(&format!(
            "start keyboard={} mouse={}",
            config.devices.keyboard, config.devices.mouse
        ));
        self.after(300, Event::Ready);
        Ok(())
    }
    fn set_name(&mut self, name: &str) -> Result<(), ErrorCode> {
        self.log(&format!("name {name}"));
        Ok(())
    }
    fn set_security(&mut self, security: Security) {
        self.security = security;
    }
    fn set_pairing(&mut self, enabled: bool) {
        let starting = enabled && !self.pairing;
        self.pairing = enabled;
        if starting && !self.connected {
            self.after(1500, Event::Connected(TARGET));
            if self.security == Security::Passkey {
                self.after(1600, Event::PasskeyRequest);
            } else {
                self.secure(1600);
            }
        }
    }
    fn passkey(&mut self, passkey: Option<u32>) -> Result<(), ErrorCode> {
        if passkey.is_some() {
            self.secure(200);
        } else {
            self.after(100, Event::PairingComplete { success: false });
            self.after(150, Event::Disconnected);
        }
        Ok(())
    }
    fn disconnect(&mut self) -> Result<(), ErrorCode> {
        self.after(100, Event::Disconnected);
        Ok(())
    }
    fn targets(&mut self) -> Vec<Target> {
        self.targets.clone()
    }
    fn max_targets(&self) -> u32 {
        8
    }
    fn forget(&mut self, target: Target) -> Result<(), ErrorCode> {
        self.targets.retain(|t| *t != target);
        if self.connected {
            self.after(50, Event::Disconnected);
        }
        Ok(())
    }
    fn set_battery(&mut self, level: u8) {
        self.log(&format!("battery {level}"));
    }
    fn can_send(&mut self) -> bool {
        self.connected && self.secured
    }
    fn send(&mut self, report: &Encoded) -> Result<(), ErrorCode> {
        let data = report.data();
        let hex: Vec<String> = data.iter().map(|b| format!("{b:02x}")).collect();
        let channel = match report.channel {
            Channel::Input(id) => format!("input{id}"),
            Channel::BootKeyboard => "boot-keyboard".into(),
            Channel::BootMouse => "boot-mouse".into(),
        };
        self.log(&format!("report {channel} {}", hex.join(" ")));
        // The Target toggles Caps Lock when the key goes down.
        if report.channel == Channel::Input(1) {
            let caps = data[2..].contains(&0x39);
            if caps && !self.caps {
                self.caps_lock = !self.caps_lock;
                let leds = if self.caps_lock { 2 } else { 0 };
                self.events.push_back(Event::Output {
                    report_id: 1,
                    data: heapless::Vec::from_slice(&[leds]).unwrap(),
                });
            }
            self.caps = caps;
        }
        Ok(())
    }
    fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }
}
/// A state directory the simulator made for itself, removed when it exits.
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() {
    let (state, _temporary) = match std::env::var_os("DEBONAIR_SIM_STATE") {
        Some(path) => (PathBuf::from(path), None),
        None => {
            let started = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |t| t.as_nanos());
            let path =
                std::env::temp_dir().join(format!("debonair-sim-{}-{started}", std::process::id()));
            (path.clone(), Some(Temporary(path)))
        }
    };
    let mut store = store::opened(Ok(Files(state)));
    let log = std::env::var_os("DEBONAIR_SIM_LOG").and_then(|path| {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    let mut radio = Simulated {
        now: 0,
        scheduled: Vec::new(),
        events: VecDeque::new(),
        security: Security::JustWorks,
        pairing: false,
        connected: false,
        secured: false,
        targets: Vec::new(),
        caps: false,
        caps_lock: false,
        log,
    };
    let mut app = App::new(
        Build {
            version: env!("CARGO_PKG_VERSION"),
            board: "simulator",
            model: "Simulator",
            development: true,
            adapter_id: "SIMULATOR".into(),
        },
        &mut store,
    );
    app.start(&mut radio);
    app.session(true);

    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stdin.read(&mut buffer) {
            if n == 0 || sender.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut stdout = std::io::stdout().lock();
    let mut output = [0u8; 4096];
    // Bytes the application has not taken yet, offered again once its output has been written.
    let mut input = Vec::new();
    loop {
        let now = started.elapsed().as_millis() as u64;
        if input.is_empty() {
            match receiver.recv_timeout(Duration::from_millis(2)) {
                Ok(bytes) => input = bytes,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let taken = app.receive(&input, &mut radio, &mut store, now);
        input.drain(..taken);
        radio.tick(now);
        while let Some(event) = radio.next_event() {
            app.radio_event(event, &mut radio);
        }
        let action = app.poll(&mut radio, now);
        loop {
            let n = app.output(&mut output);
            if n == 0 {
                break;
            }
            if stdout
                .write_all(&output[..n])
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return;
            }
        }
        if let Some(action) = action {
            radio.log(&format!("{action:?}"));
            return;
        }
    }
}
