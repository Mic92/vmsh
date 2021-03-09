use clap::{crate_authors, crate_version, value_t, App, AppSettings, Arg, ArgMatches, SubCommand};
use nix::unistd::Pid;
use vmsh::inspect::InspectOptions;
use vmsh::{attach, coredump, inspect};

fn pid_arg(index: u64) -> Arg<'static, 'static> {
    Arg::with_name("pid")
        .help("Pid of the hypervisor we get the information from")
        .required(true)
        .index(index)
}

fn parse_pid_arg(args: &ArgMatches) -> Pid {
    let pid = value_t!(args, "pid", i32).unwrap_or_else(|e| e.exit());

    Pid::from_raw(pid)
}

fn inspect(args: &ArgMatches) {
    let opts = InspectOptions {
        pid: parse_pid_arg(&args),
    };

    if let Err(err) = inspect::inspect(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn attach(args: &ArgMatches) {
    let opts = InspectOptions {
        pid: parse_pid_arg(&args),
    };

    if let Err(err) = attach::attach(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn coredump(args: &ArgMatches) {
    let opts = InspectOptions {
        pid: parse_pid_arg(&args),
    };

    if let Err(err) = coredump::generate_coredump(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn main() {
    let inspect_command = SubCommand::with_name("inspect")
        .about("Inspect a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1));

    let attach_command = SubCommand::with_name("attach")
        .about("Attach to a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1));

    let coredump_command = SubCommand::with_name("coredump")
        .about("Get a coredump of a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1));

    let main_app = App::new("vmsh")
        .about("Enter and execute in a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(inspect_command)
        .subcommand(attach_command)
        .subcommand(coredump_command);

    let matches = main_app.get_matches();
    match matches.subcommand() {
        ("inspect", Some(sub_matches)) => inspect(&sub_matches),
        ("attach", Some(sub_matches)) => attach(&sub_matches),
        ("coredump", Some(sub_matches)) => coredump(&sub_matches),
        ("", None) => unreachable!(), // beause of AppSettings::SubCommandRequiredElseHelp
        _ => unreachable!(),
    }
}
