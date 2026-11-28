//! Kalıcı küçük tanılama günlüğü: `%APPDATA%\com.proxd.akillidock\flyout.log`.
//!
//! Açılışta son ~300 satıra kısaltılır. Yalnızca ele alınan tuşlar (medya /
//! ses / kilit) ve kart olayları yazılır; normal yazı tuşları asla.
//! Kanca geri çağrısında (`hook_proc`) çağrılmaz: dosya G/Ç'si kancayı
//! zaman aşımına sokabilir.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64};

const FILE: &str = "flyout.log";
const KEEP_LINES: usize = 300;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Sistem açılışından beri geçen süre (sn).
pub fn uptime_secs() -> u64 {
    (unsafe { GetTickCount64() }) / 1000
}

/// Günlüğün bulunduğu klasör (ayar klasörü).
pub fn dir() -> Option<PathBuf> {
    PATH.get().and_then(|p| p.parent().map(PathBuf::from))
}

pub fn init(dir: PathBuf) {
    let path = dir.join(FILE);
    let _ = fs::create_dir_all(&dir);
    if let Ok(text) = fs::read_to_string(&path) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > KEEP_LINES {
            let kept = lines[lines.len() - KEEP_LINES..].join("\n");
            let _ = fs::write(&path, format!("{kept}\n"));
        }
    }
    if PATH.set(path).is_err() {
        return;
    }
    log(&format!(
        "---- süreç başladı (pid {}, sürüm {}, sistem açılalı {} sn)",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        uptime_secs()
    ));
}

pub fn log(msg: &str) {
    let Some(path) = PATH.get() else { return };
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [up {}s] {msg}\n",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
        uptime_secs()
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}
