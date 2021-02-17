use nix::sys::mman::MapFlags;

pub struct Mapping {
    start: u64,
    stop: u64,
    flags: MapFlags,
}
