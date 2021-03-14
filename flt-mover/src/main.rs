use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::prelude::*;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::*;

use anyhow::*;
use log::*;
use notify::{raw_watcher, RecursiveMode, Watcher};
use simplelog::*;
use structopt::StructOpt;

/// Monitors a directory and moves .flt files out from under BMS
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    /// Switch to this working directory (e.g., BMS/User/Acmi)
    /// before doing anything else.
    #[structopt(short = "C", long)]
    #[structopt(verbatim_doc_comment)]
    directory: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::from_args();
    init_logger(args.verbose)?;

    if let Some(change_to) = args.directory {
        env::set_current_dir(&change_to).with_context(|| {
            format!("Couldn't set working directory to {}", change_to.display())
        })?;
    }

    rename_flts()?;

    let (tx, rx) = channel();
    let mut watcher = raw_watcher(tx).unwrap();
    watcher
        .watch(env::current_dir()?, RecursiveMode::NonRecursive)
        .context("Couldn't start watching the current directory")?;

    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            // We don't actually care what the event was;
            // rescanning is cheap.
            Ok(_) | Err(RecvTimeoutError::Timeout) => rename_flts()?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn find_first_flt() -> Result<Option<PathBuf>> {
    for f in fs::read_dir(env::current_dir()?)? {
        let entry = f?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("flt")) && entry.metadata()?.is_file() {
            trace!("Found .flt file {}", path.display());
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn rename_flts() -> Result<()> {
    while let Some(flt) = find_first_flt()? {
        rename_flt(flt)?;
    }
    Ok(())
}

fn rename_flt(to_rename: PathBuf) -> Result<()> {
    let mut rename_to = find_unique_rename(to_rename.clone());

    debug!("Trying to rename {}...", to_rename.display());
    match fs::rename(&to_rename, &rename_to) {
        Ok(()) => {
            info!("Renamed {} to {}", to_rename.display(), rename_to.display());
            return Ok(());
        }
        // Windows error 32: The file is being used by another process.
        Err(e) if e.raw_os_error() == Some(32) => {
            debug!(
                "Permission denied renaming {}. Waiting for BMS to release the file...",
                to_rename.display()
            );
        }
        other_error => return other_error.context("Renaming failed unexpectedly"),
    }

    // Try to open the .flt file in a loop - there's a brief window when BMS
    // finishes writing it before it opens it back up to convert to VHS.
    let mut flt_fh = loop {
        // This seems a bit long, but it's what acmi-compiler does.
        std::thread::sleep(std::time::Duration::from_millis(250));

        let open_result = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&to_rename);

        match open_result {
            Ok(fh) => {
                trace!("Opened {}", to_rename.display());
                break fh;
            }
            // Windows error 32: The file is being used by another process.
            Err(e) if e.raw_os_error() == Some(32) => {
                trace!(
                    "Couldn't open {}. Waiting for BMS to release it...",
                    to_rename.display()
                );
            }
            Err(other_error) => {
                return Err(Error::from(other_error)
                    .context(format!("Couldn't open {}", to_rename.display())))
            }
        }
    };

    // Closing the .flt file handle and doing a rename is a bit racy -
    // it assumes that BMS has given up on trying to open it in the meantime.
    // Instead, just do a copy.

    // It might have been a bit. Let's make sure our rename target
    // is still a unique name.
    rename_to = find_unique_rename(to_rename.clone());
    let mut renamed_fh = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&rename_to)
        .with_context(|| format!("Couldn't open {} for copying", rename_to.display()))?;

    io::copy(&mut flt_fh, &mut renamed_fh)?;
    renamed_fh.flush()?;

    // Copy successful!
    info!("Renamed {} to {}", to_rename.display(), rename_to.display());
    drop(flt_fh);
    fs::remove_file(to_rename)?;

    Ok(())
}

// _TOCTOU: The Function_, but let's assume nothing's making a bunch of
// `.flt.moved.N` filenames as we go :P
fn find_unique_rename(to_rename: PathBuf) -> PathBuf {
    let mut new_name_base = to_rename.into_os_string();
    new_name_base.push(".moved");
    if !Path::new(&new_name_base).exists() {
        return PathBuf::from(new_name_base);
    }
    let mut count = 1;
    loop {
        let mut numbered_name = new_name_base.clone();
        numbered_name.push(&format!(".{}", count));
        if !Path::new(&numbered_name).exists() {
            return PathBuf::from(numbered_name);
        }
        count += 1;
    }
}

/// Set up simplelog to spit messages to stderr.
fn init_logger(verbosity: u8) -> Result<()> {
    let mut builder = ConfigBuilder::new();
    // Shut a bunch of stuff off - we're just spitting to stderr.
    builder.set_location_level(LevelFilter::Trace);
    builder.set_target_level(LevelFilter::Off);
    builder.set_thread_level(LevelFilter::Off);
    builder.set_time_level(LevelFilter::Off);

    let level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let config = builder.build();

    TermLogger::init(level, config.clone(), TerminalMode::Stderr)
        .or_else(|_| SimpleLogger::init(level, config))
        .context("Couldn't init logger")
}
