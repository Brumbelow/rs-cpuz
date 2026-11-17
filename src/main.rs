//! rs-cpuz - a system profiler for Windows.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
compile_error!("rs-cpuz reads the processor through CPUID and RDTSC and builds for x86 and x86-64 only");

mod bench;
mod brand;
mod cli;
mod config;
mod demo;
mod hw;
mod model;
mod report;
mod ui;

use std::io;
use std::path::Path;
use std::process::ExitCode;

use windows::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
use windows::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};

use cli::Command;
use config::Config;
use model::{ClockSnapshot, SystemInfo};

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            attach_parent_console();
            eprintln!("{message}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    let cfg = Config::load();
    let base = config::exe_dir();

    let result = match &args.command {
        Command::Help => {
            attach_parent_console();
            println!("{}", cli::USAGE);
            Ok(())
        }
        Command::Gui => {
            let code = ui::run(&cfg, &ui::Options { core: args.core, demo: args.demo });
            return ExitCode::from(u8::try_from(code).unwrap_or(1));
        }
        Command::Txt(name) => {
            let (sys, clocks) = probe(&cfg, args.demo);
            let path = cli::output_path(&base, name, &["txt"]);
            report::write_txt(&sys, Some(&clocks), &path).map_err(|error| failure(&path, &error))
        }
        Command::Html(name) => {
            let (sys, clocks) = probe(&cfg, args.demo);
            let path = cli::output_path(&base, name, &["htm", "html"]);
            report::write_html(&sys, Some(&clocks), &path).map_err(|error| failure(&path, &error))
        }
        Command::Snap(dir) => {
            let (sys, clocks) = probe(&cfg, args.demo);
            let dir = base.join(dir);
            std::fs::create_dir_all(&dir)
                .and_then(|()| ui::snapshot(&sys, &clocks, &cfg, args.core, &dir))
                .map(|_| ())
                .map_err(|error| failure(&dir, &error))
        }
        Command::Dump => {
            let (sys, clocks) = probe(&cfg, args.demo);
            attach_parent_console();
            println!("{sys:#?}\n{clocks:#?}");
            Ok(())
        }
        Command::Bench => run_bench(&cfg, &base, args.demo),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            attach_parent_console();
            eprintln!("{}: {message}", brand::APP_NAME);
            ExitCode::FAILURE
        }
    }
}

/// Connects standard output and standard error to the console the program was started from.
///
/// Release builds are linked for the GUI subsystem, so a shell starts them without a
/// console and whatever the command-line modes print would be lost. A stream that was
/// redirected to a file or a pipe is left alone: attaching points every stream at the
/// console, so the redirected ones are put back afterwards. Does nothing when there is
/// no console to attach to, or when the program has one already.
fn attach_parent_console() {
    let streams = [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
    // SAFETY: none of the calls takes a pointer. The handles are only asked for their
    // type and handed back as they were received; a failing call changes nothing.
    unsafe {
        let redirected = streams.map(|stream| {
            GetStdHandle(stream).ok().filter(|&handle| GetFileType(handle) != FILE_TYPE_UNKNOWN)
        });
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        for (stream, handle) in streams.into_iter().zip(redirected) {
            if let Some(handle) = handle {
                let _ = SetStdHandle(stream, handle);
            }
        }
    }
}

/// Names the file or directory an operation failed on, as in `C:\out\report.txt: Access is denied.`
fn failure(path: &Path, error: &io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// Detects the hardware (or loads the sample system) and takes one clock sample.
fn probe(cfg: &Config, demo: bool) -> (SystemInfo, ClockSnapshot) {
    if demo {
        return (demo::system(), demo::clocks());
    }
    let sys = hw::detect(cfg, &mut |_| {});
    let clocks = hw::clocks::ClockMonitor::new(&sys.cpu, sys.smbios.as_ref()).sample();
    (sys, clocks)
}

/// Runs the benchmark headless and writes the scores to `<computer name>.txt`.
///
/// With `demo` the file carries the names of the sample system; the scores are
/// those of this machine all the same, measured on all of its logical processors.
fn run_bench(cfg: &Config, base: &Path, demo: bool) -> Result<(), String> {
    let (sys, _) = probe(cfg, demo);
    let threads = bench::thread_limit((!demo).then_some(&sys.cpu));
    let result = bench::run_blocking(threads).map_err(|error| error.to_string())?;
    let name = if sys.os.computer_name.is_empty() { "benchmark" } else { &sys.os.computer_name };
    let text = format!(
        "{} benchmark {}\r\n\r\nProcessor\t\t{}\r\nSingle thread\t\t{:.1}\r\nMulti thread\t\t{:.1}\r\nThreads\t\t\t{}\r\nMulti thread ratio\t{:.2}\r\n",
        brand::APP_NAME,
        bench::VERSION,
        sys.cpu.specification,
        result.single,
        result.multi,
        result.threads,
        result.ratio(),
    );
    let path = cli::output_path(base, name, &["txt"]);
    std::fs::write(&path, text).map_err(|error| failure(&path, &error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_names_the_path() {
        let error = io::Error::new(io::ErrorKind::NotFound, "no such directory");
        assert_eq!(failure(Path::new(r"out\report.txt"), &error), r"out\report.txt: no such directory");
    }

    #[test]
    fn attaching_leaves_open_streams_alone() {
        // SAFETY: both calls only look at handles of this process.
        let open = || {
            [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|stream| unsafe {
                GetStdHandle(stream).ok().filter(|&handle| GetFileType(handle) != FILE_TYPE_UNKNOWN)
            })
        };
        let before = open();
        attach_parent_console();
        attach_parent_console();
        // A console, a file or a pipe the test was started with is still what it writes to.
        for (before, after) in before.into_iter().zip(open()) {
            assert!(before.is_none() || after == before, "{before:?} became {after:?}");
        }
    }
}
