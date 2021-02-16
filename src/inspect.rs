use crate::result::Result;
use nix::unistd::Pid;

pub struct InspectOptions {
    pub pid: Pid,
}

pub fn inspect(_opts: &InspectOptions) -> Result<()> {
    Ok(())
}
