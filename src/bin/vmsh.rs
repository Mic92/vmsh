use env_logger;
use std::path::PathBuf;

use clap::{
    crate_authors, crate_version, value_t, value_t_or_exit, App, AppSettings, Arg, ArgMatches,
    SubCommand,
};
use nix::unistd::Pid;

use vmsh::attach::{self, AttachOptions};
use vmsh::coredump::CoredumpOptions;
use vmsh::inspect::InspectOptions;
use vmsh::{coredump, inspect};

fn pid_arg(index: u64) -> Arg<'static, 'static> {
    Arg::with_name("pid")
        .help("Pid of the hypervisor we get the information from")
        .required(true)
        .index(index)
}

fn parse_pid_arg(args: &ArgMatches) -> Pid {
    Pid::from_raw(value_t_or_exit!(args, "pid", i32))
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
    let opts = AttachOptions {
        pid: parse_pid_arg(&args),
        backing: PathBuf::from(value_t!(args, "backing-file", String).unwrap_or_else(|e| e.exit())),
    };

    if let Err(err) = attach::attach(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn coredump(args: &ArgMatches) {
    let pid = parse_pid_arg(&args);
    let path =
        value_t!(args, "PATH", PathBuf).unwrap_or_else(|_| PathBuf::from(format!("core.{}", pid)));

    let opts = CoredumpOptions { pid, path };

    if let Err(err) = coredump::generate_coredump(&opts) {
        eprintln!("{}", err);
        std::process::exit(1);
    };
}

fn setup_logging(matches: &clap::ArgMatches) {
    if matches.is_present("verbose") {
        env_logger::Builder::new()
            .filter_level(log::LevelFilter::max())
            .init();
        return;
    }

    let loglevel = matches.value_of("loglevel");
    if let Some(level) = loglevel {
        env_logger::Builder::new().parse_filters(&level).init();
        return;
    }

    // default
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
}

fn main() {
    let inspect_command = SubCommand::with_name("inspect")
        .about("Inspect a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1));

    let attach_command = SubCommand::with_name("attach")
        .about("Attach (a block device) to a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1))
        .arg(
            Arg::with_name("backing-file")
                .short("f")
                .long("backing-file")
                .takes_value(true)
                .default_value("/dev/null")
                .help("File which shall be served as a block device."),
        );

    let coredump_command = SubCommand::with_name("coredump")
        .about("Get a coredump of a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(pid_arg(1))
        .arg(
            Arg::with_name("PATH")
                .help("path to coredump. Defaults to core.${pid}")
                .index(2),
        );

    let main_app = App::new("vmsh")
        .about("Enter and execute in a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(Arg::with_name("verbose")
             .short("v")
             .conflicts_with("loglevel")
             .help("shorthand for highest log level (-l trace)"))
        .arg(Arg::with_name("loglevel")
             .short("l")
             .takes_value(true)
             .help("Finegrained verbosity control. See docs.rs/env_logger. Examples: [error, warn, info, debug, trace]"))
        .subcommand(inspect_command)
        .subcommand(attach_command)
        .subcommand(coredump_command);

    let matches = main_app.get_matches();
    setup_logging(&matches);
    match matches.subcommand() {
        ("inspect", Some(sub_matches)) => inspect(&sub_matches),
        ("attach", Some(sub_matches)) => attach(&sub_matches),
        ("coredump", Some(sub_matches)) => coredump(&sub_matches),
        ("", None) => unreachable!(), // beause of AppSettings::SubCommandRequiredElseHelp
        _ => unreachable!(),
    }
}
