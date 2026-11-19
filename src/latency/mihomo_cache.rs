use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::os::windows::fs::{FileTypeExt, symlink_dir, symlink_file};
use std::path::Path;
use tempfile::TempDir;

// mihomo resolves cache.db, GeoIP data and relative provider paths under the same -d directory.
pub(super) fn probe_home(source: &Path) -> io::Result<Option<TempDir>> {
    let mut cache = match File::open(source.join("cache.db")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let home = tempfile::tempdir()?;
    copy_snapshot(&mut cache, &home.path().join("cache.db"))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.eq_ignore_ascii_case("cache.db") || entry.path() == home.path() {
            continue;
        }
        let link = home.path().join(name);
        let kind = entry.file_type()?;
        if kind.is_dir() || kind.is_symlink_dir() {
            symlink_dir(entry.path(), link)?;
        } else {
            symlink_file(entry.path(), link)?;
        }
    }
    Ok(Some(home))
}

fn copy_snapshot(cache: &mut File, destination: &Path) -> io::Result<()> {
    // bbolt commits a copy-on-write transaction by updating one of its first two pages.
    // Read both before copying, then verify the same transaction still owns the copied pages.
    // Windows bbolt uses system pages; 128 KiB covers two pages even at 64 KiB per page.
    const HEADER_REGION: u64 = 128 * 1024;
    let mut before = Vec::new();
    let mut after = Vec::new();
    for _ in 0..3 {
        cache.rewind()?;
        before.clear();
        cache.take(HEADER_REGION).read_to_end(&mut before)?;
        let mut output = File::create(destination)?;
        output.write_all(&before)?;
        io::copy(cache, &mut output)?;
        cache.rewind()?;
        after.clear();
        cache.take(HEADER_REGION).read_to_end(&mut after)?;
        if before == after {
            return Ok(());
        }
    }
    Err(io::Error::from(io::ErrorKind::WouldBlock))
}
