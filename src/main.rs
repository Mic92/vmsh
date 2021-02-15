use bcc::perf_event::PerfMapBuilder;
use bcc::BccError;
use bcc::{Kprobe, Kretprobe, BPF};

use core::sync::atomic::{AtomicBool, Ordering};
use std::ptr;
use std::sync::Arc;

/*
 * Basic Rust clone of `opensnoop`, from the iovisor tools.
 * https://github.com/iovisor/bcc/blob/master/tools/opensnoop.py
 *
 * Prints out the filename + PID every time a file is opened
 */

/*
 * Define the struct the BPF code writes in Rust
 * This must match the struct in `opensnoop.c` exactly.
 * The important thing to understand about the code in `opensnoop.c` is that it creates structs of
 * type `data_t` and pushes them into a buffer where our Rust code can read them.
 */
#[repr(C)]
struct data_t {
    id: u64,
    ts: u64,
    ret: libc::c_int,
    comm: [u8; 16],   // TASK_COMM_LEN
    fname: [u8; 255], // NAME_MAX
}

fn do_main(runnable: Arc<AtomicBool>) -> Result<(), BccError> {
    let args: Vec<String> = std::env::args().collect();

    let code = r#"
#include <uapi/linux/ptrace.h>
#include <uapi/linux/limits.h>
#include <linux/sched.h>
struct val_t {
    u64 id;
    u64 ts;
    char comm[TASK_COMM_LEN];
    const char *fname;
};
struct data_t {
    u64 id;
    u64 ts;
    int ret;
    char comm[TASK_COMM_LEN];
    char fname[NAME_MAX];
};
BPF_HASH(infotmp, u64, struct val_t);
BPF_PERF_OUTPUT(events);
int trace_entry(struct pt_regs *ctx, int dfd, const char __user *filename)
{
    struct val_t val = {};
    u64 id = bpf_get_current_pid_tgid();
    u32 pid = id >> 32; // PID is higher part
    u32 tid = id;       // Cast and get the lower part
    if (bpf_get_current_comm(&val.comm, sizeof(val.comm)) == 0) {
        val.id = id;
        val.ts = bpf_ktime_get_ns();
        val.fname = filename;
        infotmp.update(&id, &val);
    }
    return 0;
};
int trace_return(struct pt_regs *ctx)
{
    u64 id = bpf_get_current_pid_tgid();
    struct val_t *valp;
    struct data_t data = {};
    u64 tsp = bpf_ktime_get_ns();
    valp = infotmp.lookup(&id);
    if (valp == 0) {
        // missed entry
        return 0;
    }
    bpf_probe_read(&data.comm, sizeof(data.comm), valp->comm);
    bpf_probe_read(&data.fname, sizeof(data.fname), (void *)valp->fname);
    data.id = valp->id;
    data.ts = tsp / 1000;
    data.ret = PT_REGS_RC(ctx);
    events.perf_submit(ctx, &data, sizeof(data));
    infotmp.delete(&id);
    return 0;
}
"#;
    // compile the above BPF code!
    let mut module = BPF::new(code)?;
    // load + attach kprobes!
    Kprobe::new()
        .handler("trace_entry")
        .function("do_sys_open")
        .attach(&mut module)?;
    Kretprobe::new()
        .handler("trace_return")
        .function("do_sys_open")
        .attach(&mut module)?;

    // the "events" table is where the "open file" events get sent
    let table = module.table("events")?;
    // install a callback to print out file open events when they happen
    let mut perf_map = PerfMapBuilder::new(table, perf_data_callback).build()?;
    // print a header
    println!("{:-7} {:-16} {}", "PID", "COMM", "FILENAME");
    let start = std::time::Instant::now();
    // this `.poll()` loop is what makes our callback get called
    while runnable.load(Ordering::SeqCst) {
        perf_map.poll(200);
    }
    Ok(())
}

fn perf_data_callback() -> Box<dyn FnMut(&[u8]) + Send> {
    Box::new(|x| {
        // This callback
        let data = parse_struct(x);
        println!(
            "{:-7} {:-16} {}",
            data.id >> 32,
            get_string(&data.comm),
            get_string(&data.fname)
        );
    })
}

fn parse_struct(x: &[u8]) -> data_t {
    unsafe { ptr::read(x.as_ptr() as *const data_t) }
}

fn get_string(x: &[u8]) -> String {
    match x.iter().position(|&r| r == 0) {
        Some(zero_pos) => String::from_utf8_lossy(&x[0..zero_pos]).to_string(),
        None => String::from_utf8_lossy(x).to_string(),
    }
}

fn main() {
    let runnable = Arc::new(AtomicBool::new(true));
    let r = runnable.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Failed to set handler for SIGINT / SIGTERM");

    match do_main(runnable) {
        Err(x) => {
            eprintln!("Error: {}", x);
            std::process::exit(1);
        }
        _ => {}
    }
}
