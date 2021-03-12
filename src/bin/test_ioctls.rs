use clap::{value_t, App, Arg, SubCommand};
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

fn mmap(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );

    let tracee = vm.attach()?;
    let addr = try_with!(tracee.mmap(4), "mmap failed");
    assert_eq!(vm.read::<u32>(addr)?, 0);
    vm.write::<u32>(addr, &0xdeadbeef)?;
    assert_eq!(vm.read::<u32>(addr)?, 0xdeadbeef);

    Ok(())
}

fn subtest(name: &str) -> App {
    SubCommand::with_name(name).arg(Arg::with_name("pid").required(true).index(1))
}

fn main() {
    let app = App::new("test_ioctls")
        .about("Something between integration and unit test to be used by pytest.")
        .subcommand(subtest("mmap"))
        .subcommand(subtest("inject"));

    let matches = app.get_matches();
    let subcommand_name = matches.subcommand_name().expect("subcommad required");
    let subcommand_matches = matches.subcommand_matches(subcommand_name).expect("foo");
    let pid = value_t!(subcommand_matches, "pid", i32).unwrap_or_else(|e| e.exit());
    let pid = Pid::from_raw(pid);

    let result = match subcommand_name {
        "mmap" => mmap(pid),
        "inject" => inject(pid),
        _ => std::process::exit(2),
    };

    if let Err(err) = result {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("ok");
    }
}
