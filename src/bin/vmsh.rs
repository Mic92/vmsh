use log::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use clap::{crate_authors, crate_version, Arg, ArgAction, ArgMatches, Command};
use nix::unistd::Pid;

use vmsh::attach::{self, AttachOptions};
use vmsh::coredump::CoredumpOptions;
use vmsh::devices::USE_IOREGIONFD;
use vmsh::inspect::InspectOptions;
use vmsh::{console, coredump, inspect};

const VM_TYPES: &[&str] = &["process_id", "kubernetes", "vhive", "vhive_fc_vmid"];

fn _pid_arg(index: usize) -> Arg {
    Arg::new("pid")
        .help("Pid of the hypervisor we get the information from")
        .required(true)
        .index(index)
}

fn vmid_arg(index: usize) -> Arg {
    Arg::new("id")
        .help("VM/Hypervisor pid or pod name to target")
        .required(true)
        .index(index)
}

fn vmid_type_arg() -> Arg {
    Arg::new("type")
        .short('t')
        .long("type")
        .value_delimiter(',')
        .num_args(1)
        .value_name("TYPE")
        .help("VM id lookups to try (seperated by ','). [default: all]")
        .value_parser(clap::builder::PossibleValuesParser::new(VM_TYPES))
}

fn parse_vmid_arg(args: &ArgMatches) -> Pid {
    let mut container_types = vec![];
    if args.contains_id("type") {
        container_types = args
            .get_many::<String>("type")
            .expect("`type` is required")
            .filter_map(|t| container_pid::lookup_container_type(t))
            .collect();
    }

    let container_name = args.get_one::<String>("id").expect("`id` is required"); // safe, because container id is .required
    match container_pid::lookup_container_pid(container_name, &container_types) {
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
        Ok(pid) => Pid::from_raw(pid),
    }
}

fn command_args(index: usize) -> Arg {
    Arg::new("command")
        .help("Command to run in the VM")
        .action(ArgAction::Append)
        .required(false)
        .index(index)
}

fn inspect(args: &ArgMatches) {
    let opts = InspectOptions {
        pid: parse_vmid_arg(args),
    };

    if let Err(err) = inspect::inspect(&opts) {
        error!("{}", err);
        std::process::exit(1);
    };
}

fn attach_options(args: &ArgMatches) -> AttachOptions {
    let mut command = args
        .get_many::<String>("command")
        .unwrap_or_default()
        .collect::<Vec<_>>();
    let stage2_path = args
        .get_one::<String>("stage2-path")
        .expect("`stage2-path` is required");
    command.insert(0, stage2_path);

    AttachOptions {
        pid: parse_vmid_arg(args),
        command: command.into_iter().map(Clone::clone).collect::<Vec<_>>(),
        backing: args
            .get_one::<PathBuf>("backing-file")
            .expect("`backing-file` is required")
            .clone(),
        pts: args
            .get_one::<Option<PathBuf>>("pts")
            .map_or_else(|| None, Clone::clone),
    }
}

fn attach(args: &ArgMatches) {
    let opts = attach_options(args);
    USE_IOREGIONFD.store(
        args.get_one::<String>("mmio").expect("`mmio` is required") == "ioregionfd",
        Ordering::Release,
    );

    if let Err(err) = attach::attach(&opts) {
        error!("{}", err);
        std::process::exit(1);
    };
}

fn coredump(args: &ArgMatches) {
    let pid = parse_vmid_arg(args);
    let path = args
        .get_one::<PathBuf>("PATH")
        .map_or_else(|| PathBuf::from(format!("core.{}", pid)), Clone::clone);

    let opts = CoredumpOptions { pid, path };

    if let Err(err) = coredump::generate_coredump(&opts) {
        error!("{}", err);
        std::process::exit(1);
    };
}

fn console(args: &ArgMatches) {
    let opts = attach_options(args);
    if let Err(err) = console::console(&opts) {
        error!("{}", err);
        std::process::exit(1);
    };
}

fn setup_logging(matches: &clap::ArgMatches) {
    if matches.contains_id("verbose") {
        env_logger::Builder::new().parse_filters("debug").init();
        return;
    }

    let loglevel = matches.get_one::<String>("loglevel");
    if let Some(level) = loglevel {
        env_logger::Builder::new().parse_filters(level).init();
        return;
    }

    // default
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
}

fn cli() -> Command {
    Command::new("vmsh")
        .about("Enter and execute in a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .subcommand_required(false)
        .arg_required_else_help(true)
        .allow_external_subcommands(false)
        .arg(Arg::new("verbose")
             .short('v')
             .conflicts_with("loglevel")
             .num_args(1)
             .help("shorthand for --loglevel debug)"))
        .arg(Arg::new("loglevel")
             .short('l')
             .num_args(1)
             .help("Finegrained verbosity control. See docs.rs/env_logger. Examples: [error, warn, info, debug, trace]"))
        .subcommand(
            Command::new("inspect")
            .about("Inspect a virtual machine.")
            .version(crate_version!())
            .author(crate_authors!("\n"))
            .arg(vmid_arg(1))
            .arg(vmid_type_arg()))
        .subcommand(Command::new("attach")
                    .about("Attach (a block device) to a virtual machine.")
                    .version(crate_version!())
                    .author(crate_authors!("\n"))
                    .arg(vmid_arg(1))
                    .arg(vmid_type_arg())
                    .arg(
                        Arg::new("stage2-path")
                        .long("stage2-path")
                        .num_args(1)
                        .default_value("/dev/.vmsh")
                        .help("Path where Stage2 is written to in the VM"),
                        )
                    .arg(command_args(2))
                    .arg(
                        Arg::new("backing-file")
                        .short('f')
                        .long("backing-file")
                        .num_args(1)
                        .default_value("/dev/null")
                        .value_parser(clap::value_parser!(PathBuf))
                        .help("File which shall be served as a block device."),
                        )
                    .arg(
                        Arg::new("mmio")
                        .long("mmio")
                        .num_args(1)
                        .value_parser(clap::builder::PossibleValuesParser::new(["wrap_syscall", "ioregionfd"]))
                        .default_value("wrap_syscall")
                        .long_help("Backend used to serve Virtio MMIO memory of devices."),
                        )
                    .arg(
                        Arg::new("pts")
                        .long("pts")
                        .num_args(1)
                        .value_parser(clap::value_parser!(PathBuf))
                        .help("Pseudoterminal seat to use for the command run in the VM. Use this when interactivity is required. ")
                        )
       )
        .subcommand(
            Command::new("coredump")
                    .about("Get a coredump of a virtual machine.")
                    .version(crate_version!())
                    .author(crate_authors!("\n"))
                    .arg(vmid_arg(1))
                    .arg(vmid_type_arg())
                    .arg(
                        Arg::new("PATH")
                        .help("path to coredump. Defaults to core.${pid}")
                        .value_parser(clap::value_parser!(PathBuf))
                        .index(2)
                    )
        )
        .subcommand(
            Command::new("console")
                    .about("Uses the current console connected as potential target for vmsh")
                    .version(crate_version!())
                    .author(crate_authors!("\n"))
                    .arg(vmid_arg(1))
                    .arg(vmid_type_arg())
                    .arg(
                        Arg::new("stage2-path")
                        .long("stage2-path")
                        .num_args(1)
                        .default_value("/dev/.vmsh")
                        .help("Path where Stage2 is written to in the VM"),
                        )
                    .arg(command_args(2))
                    .arg(
                        Arg::new("backing-file")
                        .short('f')
                        .long("backing-file")
                        .num_args(1)
                        .default_value("/dev/null")
                        .help("File which shall be served as a block device."),
                        )
                    .arg(
                        Arg::new("pts")
                        .long("pts")
                        .num_args(1)
                        .help("Pseudoterminal seat to use for the command run in the VM. Use this when interactivity is required. ")
                    )
        )
}

fn main() {
    let matches = cli().get_matches();
    setup_logging(&matches);
    match matches.subcommand() {
        Some(("inspect", sub_matches)) => inspect(sub_matches),
        Some(("attach", sub_matches)) => attach(sub_matches),
        Some(("coredump", sub_matches)) => coredump(sub_matches),
        Some(("console", sub_matches)) => console(sub_matches),
        Some((_, _)) => unreachable!(),
        None => unreachable!(),
    }
}

#[cfg(test)]
mod tests {

    use super::VM_TYPES;
    use container_pid::AVAILABLE_CONTAINER_TYPES;

    #[test]
    fn test_container_pid_compat() {
        for t in VM_TYPES {
            assert!(AVAILABLE_CONTAINER_TYPES.contains(t));
        }
    }
}
