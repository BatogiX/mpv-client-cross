use log::{Level, Log, Metadata, Record, SetLoggerError};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write as _};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::{Handle, Node};

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

            let elapsed = log_file.start_time.elapsed().unwrap_or_default().as_secs_f64();
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

    let path_log_file = mp.get_property::<String>("log-file").unwrap();
    let path_log_file = if path_log_file.starts_with('~') {
        let node = mp.command_ret(["expand-path", &path_log_file]).unwrap();

        let Node::String(expanded) = node else {
            unreachable!("'expand-path' always return a String variant")
        };

        Some(expanded)
    } else if path_log_file.is_empty() {
        None
    } else {
        Some(path_log_file)
    };

    let log_file = path_log_file.map(|mut path_log_file| {
        let now = SystemTime::now();
        let last_line = read_last_line(&path_log_file).unwrap();

        let last_time = Duration::from_secs_f64(
            last_line[1..=8]
                .trim_start()
                .parse::<f64>()
                .expect("cannot parse to f64"),
        );

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
        let start_time = now - last_time;
        LogFile { file, start_time }
    });

    let logger = Box::leak(Box::new(MpvLogger { module, log_file }));
    log::set_logger(logger)?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}

fn read_last_line<P: AsRef<Path>>(file_path: P) -> io::Result<String> {
    let mut file = File::open(file_path)?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(String::new());
    }

    let tail_size = 1024.min(file_size) as usize;
    let mut buffer = vec![0; tail_size];
    file.seek(SeekFrom::End(-(tail_size as i64)))?;
    file.read_exact(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer);
    let last_line = text.lines().rfind(|s| !s.trim().is_empty()).unwrap_or("");
    Ok(last_line.to_owned())
}
