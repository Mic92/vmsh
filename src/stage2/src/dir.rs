use std::fs::create_dir_all;
use std::{io, path::Path};

pub fn mkdir_p<P: AsRef<Path>>(path: &P) -> io::Result<()> {
    if let Err(e) = create_dir_all(path) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(e);
        }
    }
    Ok(())
}
