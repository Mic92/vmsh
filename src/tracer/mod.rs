use crate::proc::Mapping;
use crate::ptrace;

pub struct Tracer {
    pub process_idx: usize,
    pub threads: Vec<ptrace::Thread>,
    pub vcpu_map: Mapping, // TODO support multiple cpus
}

impl Tracer {
    #[allow(dead_code)]
    fn main_thread(&self) -> &ptrace::Thread {
        &self.threads[self.process_idx]
    }

    #[allow(dead_code)]
    fn main_thread_mut(&mut self) -> &mut ptrace::Thread {
        &mut self.threads[self.process_idx]
    }
}
