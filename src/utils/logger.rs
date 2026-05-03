use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

static LOGGER: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn log_file() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_default()
        .join("smartboard.log")
}

pub fn init() {
    let path = log_file();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .truncate(false)
        .open(&path)
        .expect("failed to open log file");
    *LOGGER.lock().unwrap() = Some(file);
    crate::log_info!("logger initialized: {}", path.display());
}

fn timestamp() -> String {
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", t.as_secs(), t.subsec_millis())
}

pub fn write_log(level: &str, msg: &str) {
    let line = format!("{} [{}] {}\n", timestamp(), level, msg);
    if let Ok(mut guard) = LOGGER.lock() {
        if let Some(ref mut f) = *guard {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
    // also emit to console
    if level == "ERROR" || level == "WARN " {
        eprintln!("[{}] {}", level.trim(), msg);
    } else {
        println!("[{}] {}", level.trim(), msg);
    }
}
