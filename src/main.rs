use argparse::{ArgumentParser, List, Store};
use libc::pid_t;
use nix::unistd::Pid;
use std::io::{stderr, stdout};
use std::str::FromStr;

use crate::inspect::InspectOptions;

mod attach;
mod cpu;
mod device;
mod gdb_break;
mod inject_syscall;
mod inspect;
mod kvm;
mod kvm_ioctls;
mod kvm_memslots;
mod page_math;
mod proc;
mod ptrace;
mod result;

#[allow(non_camel_case_types)]
#[derive(Debug)]
enum Command {
    inspect,
    attach,
}

impl FromStr for Command {
    type Err = ();
    fn from_str(src: &str) -> std::result::Result<Command, ()> {
        match src {
            "inspect" => Ok(Command::inspect),
            "attach" => Ok(Command::attach),
            _ => Err(()),
        }
    }
}

fn parse_inspect_args(args: Vec<String>) -> InspectOptions {
    let mut options = InspectOptions {
        pid: Pid::from_raw(0),
    };
    let mut hypervisor_pid: pid_t = 0;
    {
        let mut ap = ArgumentParser::new();
        ap.set_description("inspect vm");
        ap.refer(&mut hypervisor_pid).required().add_argument(
            "pid",
            Store,
            "Pid of the hypervisor we get the information from",
        );
        match ap.parse(args, &mut stdout(), &mut stderr()) {
            Ok(()) => {}
            Err(x) => {
                std::process::exit(x);
            }
        }
    }
    options.pid = Pid::from_raw(hypervisor_pid);

    options
}

fn inspect_command(args: Vec<String>) {
    let opts = parse_inspect_args(args);
    if let Err(err) = inspect::inspect(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn attach_command(args: Vec<String>) {
    let opts = parse_inspect_args(args);
    if let Err(err) = attach::attach(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn main() {
    let mut subcommand = Command::inspect;
    let mut args = vec![];
    {
        let mut ap = ArgumentParser::new();
        ap.set_description("Enter or executed in container");
        ap.refer(&mut subcommand).required().add_argument(
            "command",
            Store,
            r#"Command to run (either "inspect")"#,
        );
        ap.refer(&mut args)
            .add_argument("arguments", List, r#"Arguments for command"#);

        ap.stop_on_first_argument(true);
        ap.parse_args_or_exit();
    }

    args.insert(0, format!("subcommand {:?}", subcommand));

    match subcommand {
        Command::inspect => inspect_command(args),
        Command::attach => attach_command(args),
    }
}
