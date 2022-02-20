use log::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use nix::unistd::Pid;

use vmsh::attach::{self, AttachOptions};
use vmsh::coredump::CoredumpOptions;
use vmsh::devices::USE_IOREGIONFD;
use vmsh::inspect::InspectOptions;
use vmsh::{console, coredump, inspect};

const VM_TYPES: &[&str] = &["process_id", "kubernetes", "vhive", "vhive_fc_vmid"];

fn _pid_arg(index: usize) -> Arg<'static> {
    Arg::new("pid")
        .help("Pid of the hypervisor we get the information from")
        .required(true)
        .index(index)
}

fn _parse_pid_arg(args: &ArgMatches) -> Pid {
    Pid::from_raw(args.value_of_t_or_exit("pid"))
}

fn vmid_arg(index: usize) -> Arg<'static> {
    Arg::new("id")
        .help("VM/Hypervisor pid or pod name to target")
        .required(true)
        .index(index)
}

fn vmid_type_arg() -> Arg<'static> {
    Arg::new("type")
        .short('t')
        .long("type")
        .takes_value(true)
        .forbid_empty_values(false)
        .require_delimiter(true)
        .value_delimiter(',')
        .value_name("TYPE")
        .help("VM id lookups to try (seperated by ','). [default: all]")
        .possible_values(VM_TYPES)
}

fn parse_vmid_arg(args: &ArgMatches) -> Pid {
    let mut container_types = vec![];
    if args.is_present("type") {
        let types = args
            .values_of_t::<String>("type")
            .unwrap_or_else(|e| e.exit());
        container_types = types
            .into_iter()
            .filter_map(|t| container_pid::lookup_container_type(&t))
            .collect();
    }

    let container_name = args.value_of("id").unwrap().to_string(); // safe, because container id is .required
    match container_pid::lookup_container_pid(&container_name, &container_types) {
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
        Ok(pid) => Pid::from_raw(pid),
    }
}

fn command_args(index: usize) -> Arg<'static> {
    Arg::new("command")
        .help("Command to run in the VM")
        .multiple_occurrences(true)
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
    let mut command = args.values_of_t("command").unwrap_or_else(|_| vec![]);
    let stage2_path = args.value_of_t_or_exit::<String>("stage2-path");
    command.insert(0, stage2_path);

    AttachOptions {
        pid: parse_vmid_arg(args),
        command,
        backing: PathBuf::from(args.value_of_t_or_exit::<String>("backing-file")),
        pts: args.value_of_t::<String>("pts").ok().map(PathBuf::from),
    }
}

fn attach(args: &ArgMatches) {
    let opts = attach_options(args);
    USE_IOREGIONFD.store(
        args.value_of_t_or_exit::<String>("mmio") == "ioregionfd",
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
        .value_of_t("PATH")
        .unwrap_or_else(|_| PathBuf::from(format!("core.{}", pid)));

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
    if matches.is_present("verbose") {
        env_logger::Builder::new().parse_filters("debug").init();
        return;
    }

    let loglevel = matches.value_of("loglevel");
    if let Some(level) = loglevel {
        env_logger::Builder::new().parse_filters(level).init();
        return;
    }

    // default
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
}

fn main() {
    let inspect_command = App::new("inspect")
        .about("Inspect a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(vmid_arg(1))
        .arg(vmid_type_arg());

    let attach_command = App::new("attach")
        .about("Attach (a block device) to a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(vmid_arg(1))
        .arg(vmid_type_arg())
        .arg(
            Arg::new("stage2-path")
                .long("stage2-path")
                .takes_value(true)
                .default_value("/dev/.vmsh")
                .help("Path where Stage2 is written to in the VM"),
        )
        .arg(command_args(2))
        .arg(
            Arg::new("backing-file")
                .short('f')
                .long("backing-file")
                .takes_value(true)
                .default_value("/dev/null")
                .help("File which shall be served as a block device."),
        )
        .arg(
            Arg::new("mmio")
                .long("mmio")
                .takes_value(true)
                .possible_values(&["wrap_syscall", "ioregionfd"])
                .default_value("wrap_syscall")
                .long_help("Backend used to serve Virtio MMIO memory of devices."),
        )
        .arg(
            Arg::new("pts")
                .long("pts")
                .takes_value(true)
                .help("Pseudoterminal seat to use for the command run in the VM. Use this when interactivity is required. "),
        );

    let coredump_command = App::new("coredump")
        .about("Get a coredump of a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(vmid_arg(1))
        .arg(vmid_type_arg())
        .arg(
            Arg::new("PATH")
                .help("path to coredump. Defaults to core.${pid}")
                .index(2),
        );

    let console_command = App::new("console")
        .about("Uses the current console connected as potential target for vmsh")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(vmid_arg(1))
        .arg(vmid_type_arg())
        .arg(
            Arg::new("stage2-path")
                .long("stage2-path")
                .takes_value(true)
                .default_value("/dev/.vmsh")
                .help("Path where Stage2 is written to in the VM"),
        )
        .arg(command_args(2))
        .arg(
            Arg::new("backing-file")
                .short('f')
                .long("backing-file")
                .takes_value(true)
                .default_value("/dev/null")
                .help("File which shall be served as a block device."),
        )
        .arg(
            Arg::new("pts")
                .long("pts")
                .takes_value(true)
                .help("Pseudoterminal seat to use for the command run in the VM. Use this when interactivity is required. "),
        );

    let main_app = App::new("vmsh")
        .about("Enter and execute in a virtual machine.")
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(Arg::new("verbose")
             .short('v')
             .conflicts_with("loglevel")
             .help("shorthand for --loglevel debug)"))
        .arg(Arg::new("loglevel")
             .short('l')
             .takes_value(true)
             .help("Finegrained verbosity control. See docs.rs/env_logger. Examples: [error, warn, info, debug, trace]"))
        .subcommands([
            inspect_command,
            attach_command,
            coredump_command,
            console_command
        ]);

    let matches = main_app.get_matches();
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
            assert!(AVAILABLE_CONTAINER_TYPES.contains(&t));
        }
    }
}
