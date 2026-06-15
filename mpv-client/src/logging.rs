use crate::Handle;
use log::{Level, Log, Metadata, Record, SetLoggerError};
use std::{
    fs::File,
    io::Write as _,
    sync::Mutex,
    time::{Duration, SystemTime},
};

pub struct MpvLogger {
    module: String,
    log_file: Option<LogFile>,
}

struct LogFile {
    file: Mutex<File>,
    start_time: SystemTime,
}

impl Log for MpvLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let (color_start, color_end) = match record.level() {
            Level::Error => ("\x1b[31m", "\x1b[0m"),
            Level::Warn => ("\x1b[33m", "\x1b[0m"),
            _ => ("", ""),
        };

        let log_message = format!("[{}] {}\n", self.module, record.args());
        eprint!("{color_start}{log_message}{color_end}");

        if let Some(log_file) = &self.log_file {
            let level_str = match record.level() {
                Level::Error => "e",
                Level::Warn => "w",
                Level::Info => "i",
                Level::Debug => "d",
                Level::Trace => "v",
            };

            let elapsed = log_file
                .start_time
                .elapsed()
                .expect("start_time is valid")
                .as_secs_f64();

            let log_message = format!("[{elapsed:>8.3}][{level_str}]{log_message}");

            if let Ok(mut file) = log_file.file.lock() {
                let _ = file.write_all(log_message.as_bytes());
            }
        }
    }

    fn flush(&self) {}
}

pub fn init(mp: &Handle) -> Result<(), SetLoggerError> {
    let module = mp.name().to_owned();
    let path_log_file: String = mp.get_property("log-file").expect("log-file property must exist");
    let path_log_file = if path_log_file.starts_with('~') {
        let expanded: String = mp
            .command_ret(["expand-path", &path_log_file])
            .expect("expand-path must succeed");

        Some(expanded)
    } else if path_log_file.is_empty() {
        None
    } else {
        Some(path_log_file)
    };

    let log_file = path_log_file.map(|mut path_log_file| {
        let now = SystemTime::now();

        let internal_now =
            Duration::from_micros(u64::try_from(mp.get_time_us()).expect("mpv internal time is negative or invalid"));

        let suffix = format!("-{module}");
        match path_log_file.rfind('.') {
            Some(dot_index) => match path_log_file.rfind(['/', '\\']) {
                Some(slash_index) => {
                    if dot_index < slash_index {
                        path_log_file.push_str(&suffix);
                    } else {
                        path_log_file.insert_str(dot_index, &suffix);
                    }
                }
                None => path_log_file.insert_str(dot_index, &suffix),
            },
            None => path_log_file.push_str(&suffix),
        }

        let file = Mutex::new(File::create(&path_log_file).expect("failed to create log file"));
        let start_time = now - internal_now;
        LogFile { file, start_time }
    });

    let logger = Box::new(MpvLogger { module, log_file });
    log::set_boxed_logger(logger)?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}
