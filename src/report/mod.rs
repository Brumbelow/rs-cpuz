//! Text and HTML reports.
//!
//! Both formats render the same document: `doc` turns the model into sections
//! of lines (built by `processor`, `board`, `devices`, `dmi` and `storage`), and
//! `txt` and `html` lay those lines out.

mod board;
mod devices;
mod dmi;
mod doc;
mod fmt;
mod html;
mod processor;
mod storage;
mod txt;

use std::io;
use std::path::Path;

use crate::model::{ClockSnapshot, SystemInfo};

pub use html::html;
pub use txt::txt;

pub fn write_txt(sys: &SystemInfo, clocks: Option<&ClockSnapshot>, path: &Path) -> io::Result<()> {
    std::fs::write(path, txt(sys, clocks))
}

pub fn write_html(sys: &SystemInfo, clocks: Option<&ClockSnapshot>, path: &Path) -> io::Result<()> {
    std::fs::write(path, html(sys, clocks))
}
