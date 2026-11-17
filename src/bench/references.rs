//! The reference file: saved benchmark results to compare a run against.
//!
//! It is plain text with one `name = single ; multi` entry per line. Blank
//! lines and lines starting with `#` or `;` are ignored, so the file can be
//! annotated and edited by hand; saving a result leaves such lines alone.
//!
//! The text is UTF-8. A hand-written file in another single-byte encoding is
//! still usable: saving carries its lines over byte for byte. A UTF-16 file is
//! not; it reads as an empty list and saving refuses to rewrite it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::Reference;

const HEADER: &str =
    "# RS-CPUZ benchmark references\r\n# name = single thread score ; multi thread score\r\n";

/// Loads saved reference results; a missing file is an empty list.
pub fn load_references(path: &Path) -> Vec<Reference> {
    fs::read(path).map(|bytes| parse(&String::from_utf8_lossy(&bytes))).unwrap_or_default()
}

/// Adds or replaces the reference named `reference.name` in the file.
///
/// Fails with `InvalidData`, leaving the file as it is, when the file holds
/// UTF-16 text: its lines cannot be told apart byte-wise.
pub fn save_reference(path: &Path, reference: &Reference) -> io::Result<()> {
    let line = format_line(reference)?;
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    if is_wide_text(&existing) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the existing file looks like UTF-16 text; save it as UTF-8 to add references to it",
        ));
    }
    write_replacing(path, &merge(&existing, reference.name.trim(), &line))
}

/// Whether `content` is text in 16-bit (or wider) code units: it starts with a
/// UTF-16 byte order mark or contains the NUL bytes that such an encoding
/// spends on every ASCII character, line ends included.
fn is_wide_text(content: &[u8]) -> bool {
    content.starts_with(&[0xFF, 0xFE]) || content.starts_with(&[0xFE, 0xFF]) || content.contains(&0)
}

/// Parses the text of a reference file. Lines that are not entries are
/// skipped, and a name that occurs again replaces its earlier entry.
fn parse(text: &str) -> Vec<Reference> {
    let mut references: Vec<Reference> = Vec::new();
    for entry in text.lines().filter_map(parse_line) {
        match references.iter_mut().find(|known| same_name(&known.name, &entry.name)) {
            Some(known) => *known = entry,
            None => references.push(entry),
        }
    }
    references
}

/// Parses one `name = single ; multi` line. The name may itself contain `=`
/// and `#`; a `#` after the scores starts a comment.
fn parse_line(line: &str) -> Option<Reference> {
    let line = line.trim_start_matches('\u{FEFF}').trim();
    if line.is_empty() || line.starts_with(['#', ';']) {
        return None;
    }
    // The scores follow the last `=` that is actually followed by two numbers.
    line.rmatch_indices('=').find_map(|(at, _)| parse_entry(&line[..at], &line[at + 1..]))
}

fn parse_entry(name: &str, scores: &str) -> Option<Reference> {
    let name = name.trim();
    let scores = scores.split_once('#').map_or(scores, |(scores, _comment)| scores);
    let (single, multi) = scores.split_once(';')?;
    let reference =
        Reference { name: name.to_string(), single: parse_score(single)?, multi: parse_score(multi)? };
    (!name.is_empty()).then_some(reference)
}

/// Parses a score, accepting a decimal comma as well as a decimal point.
fn parse_score(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse().ok().filter(|score| is_score(*score))
}

fn is_score(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Formats the line stored for `reference`, refusing anything that would not read back.
fn format_line(reference: &Reference) -> io::Result<String> {
    let name = reference.name.trim();
    let name_reads_back = !name.is_empty() && !name.starts_with(['#', ';']) && !name.contains(['\r', '\n']);
    if !name_reads_back || !is_score(reference.single) || !is_score(reference.multi) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a storable benchmark reference"));
    }
    Ok(format!("{name} = {:.1} ; {:.1}", reference.single, reference.multi))
}

/// Returns the file content `existing` with the entry called `name` replaced
/// by `line`, or with `line` appended when there is none. All other lines are
/// kept byte for byte, whatever their encoding; only the line ends are unified.
fn merge(existing: &[u8], name: &str, line: &str) -> Vec<u8> {
    let mut merged = Vec::with_capacity(existing.len() + HEADER.len() + line.len() + 2);
    let mut replaced = false;
    for current in lines(existing) {
        // A line that is not UTF-8 cannot spell `name`: it is never taken for the entry.
        let entry = std::str::from_utf8(current).ok().and_then(parse_line);
        let is_same_entry = entry.is_some_and(|entry| same_name(&entry.name, name));
        if is_same_entry && replaced {
            continue;
        }
        merged.extend_from_slice(if is_same_entry { line.as_bytes() } else { current });
        merged.extend_from_slice(b"\r\n");
        replaced |= is_same_entry;
    }
    if !replaced {
        if merged.is_empty() {
            merged.extend_from_slice(HEADER.as_bytes());
        }
        merged.extend_from_slice(line.as_bytes());
        merged.extend_from_slice(b"\r\n");
    }
    merged
}

/// The lines of `content` without their LF or CR LF ends, split the way
/// `str::lines` does it but without requiring UTF-8.
fn lines(content: &[u8]) -> impl Iterator<Item = &[u8]> {
    content.split_inclusive(|&byte| byte == b'\n').map(|line| match line.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    })
}

/// Writes `content` to a temporary file next to `path` and then renames it
/// over `path`, so that a failed write cannot destroy the existing file.
fn write_replacing(path: &Path, content: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = fs::File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(content)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory under the system temporary directory, removed again on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rs-cpuz-test-{}-{tag}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("references.txt")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn reference(name: &str, single: f64, multi: f64) -> Reference {
        Reference { name: name.to_string(), single, multi }
    }

    /// `merge` on text.
    fn merged(existing: &str, name: &str, line: &str) -> String {
        String::from_utf8(merge(existing.as_bytes(), name, line)).unwrap()
    }

    #[test]
    fn parses_entries_and_skips_everything_else() {
        let text = "\u{FEFF}# saved results\r\n\
                    \r\n\
                    ; another comment\r\n\
                    Sample CPU A = 652.3 ; 7012.8\r\n\
                    \t  Sample CPU B=500;4000   \r\n\
                    Sample CPU C = 410,5 ; 1650,25 # measured in a laptop\n\
                    not an entry\n\
                    Missing multi = 5\n\
                    Bad number = fast ; 3\n\
                    Negative = -1 ; 3\n\
                    Infinite = inf ; 3\n\
                    = 1 ; 2\n";
        assert_eq!(
            parse(text),
            [
                reference("Sample CPU A", 652.3, 7012.8),
                reference("Sample CPU B", 500.0, 4000.0),
                reference("Sample CPU C", 410.5, 1650.25),
            ]
        );
    }

    #[test]
    fn names_may_contain_separators() {
        assert_eq!(
            parse_line("Model #5 (mode=fast) = 1.5 ; 3"),
            Some(reference("Model #5 (mode=fast)", 1.5, 3.0))
        );
        assert_eq!(parse_line("a = 1 ; 2 = 3 ; 4"), Some(reference("a = 1 ; 2", 3.0, 4.0)));
        assert_eq!(parse_line("b = 1 ; 2 # note = none"), Some(reference("b", 1.0, 2.0)));
    }

    #[test]
    fn repeated_names_keep_the_last_entry() {
        let parsed = parse("First = 1 ; 2\nSecond = 3 ; 4\nFIRST = 5 ; 6\n");
        assert_eq!(parsed, [reference("FIRST", 5.0, 6.0), reference("Second", 3.0, 4.0)]);
    }

    #[test]
    fn merge_replaces_in_place_and_keeps_other_lines() {
        let existing = "# mine\nFirst = 1 ; 2\n\nbroken line\nSecond = 3 ; 4\nfirst = 9 ; 9\n";
        assert_eq!(
            merged(existing, "First", "First = 10.0 ; 20.0"),
            "# mine\r\nFirst = 10.0 ; 20.0\r\n\r\nbroken line\r\nSecond = 3 ; 4\r\n"
        );
        assert_eq!(
            merged(existing, "Third", "Third = 5.0 ; 6.0"),
            "# mine\r\nFirst = 1 ; 2\r\n\r\nbroken line\r\nSecond = 3 ; 4\r\nfirst = 9 ; 9\r\nThird = 5.0 ; 6.0\r\n"
        );
        assert_eq!(merged("", "Only", "Only = 1.0 ; 2.0"), format!("{HEADER}Only = 1.0 ; 2.0\r\n"));
        assert_eq!(merged("Last = 1 ; 2", "last", "last = 3.0 ; 4.0"), "last = 3.0 ; 4.0\r\n");
    }

    #[test]
    fn merge_splits_lines_like_the_parser() {
        for text in
            ["", "\n", "a", "a\n", "a\r\n", "a\r", "a\r\nb", "a\n\nb\n", "\r\n\r\n", "a\rb\n", "a\n\r"]
        {
            let expected: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
            assert_eq!(lines(text.as_bytes()).collect::<Vec<_>>(), expected, "{text:?}");
        }
    }

    #[test]
    fn merge_keeps_undecodable_lines_and_never_matches_them() {
        // Windows-1252 text: an e acute in the comment, a trade mark sign in the name.
        let ansi = b"# caf\xE9 machines\r\nCore\x99 box = 100 ; 200\r\n";
        // A lossy decode would read the name below out of the second line and replace it.
        let lossy = "Core\u{FFFD} box";
        let merged = merge(ansi, lossy, "Core\u{FFFD} box = 7.0 ; 8.0");
        assert_eq!(merged, [&ansi[..], "Core\u{FFFD} box = 7.0 ; 8.0\r\n".as_bytes()].concat());
    }

    #[test]
    fn rejects_references_that_would_not_read_back() {
        for bad in [
            reference("", 1.0, 2.0),
            reference("   ", 1.0, 2.0),
            reference("# name", 1.0, 2.0),
            reference("two\nlines", 1.0, 2.0),
            reference("name", f64::NAN, 2.0),
            reference("name", 1.0, f64::INFINITY),
            reference("name", -1.0, 2.0),
        ] {
            assert_eq!(format_line(&bad).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let dir = TempDir::new("missing");
        assert!(load_references(&dir.file()).is_empty());
    }

    #[test]
    fn saved_references_round_trip() {
        let dir = TempDir::new("round-trip");
        let path = dir.file();
        let first = reference("Sample CPU A", 652.3, 7012.8);
        let second = reference("Sample CPU B (8 threads)", 480.0, 3310.5);

        save_reference(&path, &first).unwrap();
        save_reference(&path, &second).unwrap();
        assert_eq!(load_references(&path), [first.clone(), second.clone()]);

        // Saving under an existing name replaces that entry where it stands.
        let updated = reference("sample cpu a", 700.1, 7500.0);
        save_reference(&path, &updated).unwrap();
        assert_eq!(load_references(&path), [updated, second]);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER), "{text:?}");
        assert_eq!(text.lines().count(), 4, "{text:?}");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1, "no temporary file may be left behind");
    }

    #[test]
    fn saving_keeps_hand_written_lines() {
        let dir = TempDir::new("hand-written");
        let path = dir.file();
        fs::write(&path, "; my machines\nOld box = 100 ; 200\n\n# the rest\n").unwrap();

        save_reference(&path, &reference("New box", 300.0, 2400.0)).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "; my machines\r\nOld box = 100 ; 200\r\n\r\n# the rest\r\nNew box = 300.0 ; 2400.0\r\n"
        );
    }

    #[test]
    fn saving_keeps_lines_in_another_encoding() {
        let dir = TempDir::new("single-byte");
        let path = dir.file();
        // Windows-1252 text, as an older editor or `Set-Content` writes it.
        fs::write(&path, b"# caf\xE9 machines\r\nCore\x99 box = 100 ; 200\r\nOld box = 1 ; 2\r\n").unwrap();

        save_reference(&path, &reference("New box", 300.0, 2400.0)).unwrap();
        save_reference(&path, &reference("old box", 5.0, 6.0)).unwrap();
        assert_eq!(
            fs::read(&path).unwrap().escape_ascii().to_string(),
            b"# caf\xE9 machines\r\nCore\x99 box = 100 ; 200\r\nold box = 5.0 ; 6.0\r\nNew box = 300.0 ; 2400.0\r\n"
                .escape_ascii()
                .to_string()
        );
        assert_eq!(load_references(&path).len(), 3);
    }

    #[test]
    fn utf16_file_is_refused_and_left_alone() {
        let dir = TempDir::new("utf-16");
        let path = dir.file();
        let text = "# mine\r\nOld box = 100 ; 200\r\n";
        let little: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let big: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let files = [
            [&[0xFF, 0xFE][..], &little].concat(),
            [&[0xFE, 0xFF][..], &big].concat(),
            // Without a byte order mark the text is even valid UTF-8, full of NUL characters.
            little.clone(),
            vec![0xFF, 0xFE],
        ];
        for content in files {
            fs::write(&path, &content).unwrap();
            let error = save_reference(&path, &reference("New box", 300.0, 2400.0)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{content:?}");
            assert_eq!(fs::read(&path).unwrap(), content);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1, "no temporary file may be left behind");
        }
    }

    #[test]
    fn failed_save_reports_the_error_and_leaves_the_file() {
        let dir = TempDir::new("failed-save");
        let path = dir.file();
        save_reference(&path, &reference("Kept", 1.0, 2.0)).unwrap();
        assert!(save_reference(&path, &reference("", 3.0, 4.0)).is_err());
        assert!(
            save_reference(&dir.0.join("no-such-dir").join("file.txt"), &reference("x", 1.0, 2.0)).is_err()
        );
        assert_eq!(load_references(&path), [reference("Kept", 1.0, 2.0)]);
    }
}
