use clap::{value_t, App, Arg};
use nix::unistd::Pid;
use simple_error::try_with;
use vmsh::result::Result;

use vmsh::kvm;

fn inject(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );

    print!("check_extensions");
    for _ in 1..100 {
        let tracee = vm.attach()?;
        try_with!(tracee.check_extension(0), "cannot query kvm extensions");
        print!(".")
    }
    println!(" ok");

    Ok(())
}

fn main() {
    let app = App::new("test_ioctls")
        .about("Something between integration and unit test to be used by pytest.")
        .arg(Arg::with_name("pid").required(true).index(1));
    let matches = app.get_matches();
    let pid = value_t!(matches, "pid", i32).unwrap_or_else(|e| e.exit());
    let pid = Pid::from_raw(pid);

    if let Err(err) = inject(pid) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}
