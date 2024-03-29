mod packages;

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use std::{fs, str};

use clap::{Parser, ValueEnum};
use distinst::chroot::Chroot;
use distinst::steps::configure::ChrootConfigurator;
use log::info;
use sudo;

use packages::{INTERACTIVE, RUNTIME, RUNTIME_CLEANUP};

const MAIN_REPO: &[&str] = &["main"];
const CODENAME: &str = "jammy";
const POPKEY: &str = "204DD8AEC33A7AFF";
const POPKEY_PATHS: [&str; 2] = [
    "/etc/apt/trusted.gpg.d/pop-keyring-2017-archive.gpg",
    "/usr/share/keyrings/pop-archive-keyring.gpg",
];

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(default_value_t = ContainerType::Runtime, value_enum)]
    /// The type of Pop!_OS container that you would like to build.
    ///
    /// There are two options "Runtime" and "Interactive"
    ///
    /// Runtime => A small container. Useful in a cloud cluster.
    ///
    /// Interactive => A similar cli environtment to Pop desktop.
    ///
    ///                Useful in a containerized development environment.
    ///
    ///
    /// Example: pop-container-builder interactive
    container: ContainerType,
    #[arg(short, long)]
    /// Additional delelopment branches to add to the container
    ///
    /// during build time. This takes the same inputs as the `apt-manage`
    ///
    /// command.
    ///
    ///
    /// Exmaple pop-contaienr-builder runtime --add master --add another-branch
    add: Vec<String>,
    #[arg(short, long)]
    /// Additional packages to install into container.
    ///
    /// Exmaple pop-contaienr-builder runtime --package firefox --package chrome-stable
    package: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum ContainerType {
    Runtime,
    Interactive,
}

impl std::fmt::Display for ContainerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerType::Runtime => write!(f, "runtime"),
            ContainerType::Interactive => write!(f, "interactive"),
        }
    }
}

fn main() -> Result<(), Errors> {
    let args = Args::parse();

    let username = get_username();
    sudo::escalate_if_needed()?;
    env_logger::Builder::from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    )
    .format_timestamp(None)
    .format_module_path(false)
    .format_indent(None)
    .format_target(false)
    .init();

    info!("Creating Container.");
    let output = Command::new("buildah").args(["from", "scratch"]).output()?;
    let len = output.stdout.len() - 1;
    let container = str::from_utf8(&output.stdout[..len])?;

    let output = Command::new("buildah")
        .args(["mount", &container])
        .output()?;
    let len = output.stdout.len() - 1;
    let mount_point = str::from_utf8(&output.stdout[..len])?;

    info!("Adding minbase to container.");
    let mut debootstrap = Command::new("debootstrap");
    debootstrap.args([
        "--variant=minbase",
        CODENAME,
        mount_point,
        "http://archive.ubuntu.com/ubuntu/",
    ]);
    watch(debootstrap)?;

    info!("Adding Pop!_OS specific changes to the container.");
    {
        let mut chroot = Chroot::new(&mount_point)?;
        chroot.env("LC_CTYPE", "en_US.UTF8");
        chroot.env("HOME", "/root");
        chroot.env("LC_ALL", "en_US.UTF8");
        let chroot = ChrootConfigurator::new(chroot);
        info!("Installing build packages");
        chroot.apt_install(&["software-properties-common"])?;
        chroot.apt_key(&POPKEY_PATHS, "keyserver.ubuntu.com", POPKEY)?;
        // Add pop os necessary ppas
        chroot.apt_add_repository(&[
            ("http://apt.pop-os.org/proprietary", CODENAME, MAIN_REPO),
            ("http://apt.pop-os.org/release", CODENAME, MAIN_REPO),
            (
                "http://apt.pop-os.org/ubuntu",
                CODENAME,
                &["main", "universe", "multiverse", "restricted"],
            ),
            (
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-security"),
                &["main", "universe", "multiverse", "restricted"],
            ),
            (
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-updates"),
                &["main", "universe", "multiverse", "restricted"],
            ),
            (
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-backports"),
                &["main", "universe", "multiverse", "restricted"],
            ),
        ])?;

        // Add additional staging branches from cli args
        for repo in args.add {
            chroot.apt_add_repository(&[(
                &format!("http://apt.pop-os.org/staging/{}", repo),
                CODENAME,
                MAIN_REPO,
            )])?;
        }

        // Update and install packages
        info!("Installing Updates.");
        chroot.apt_update()?;
        chroot.apt_upgrade()?;
        // install mandatory packages for contaienr type
        chroot.apt_install(match args.container {
            ContainerType::Runtime => &RUNTIME,
            ContainerType::Interactive => &INTERACTIVE,
        })?;
        // install optional packages from `--package` flag
        chroot.apt_install(&args.package.iter().map(|p| &**p).collect::<Vec<&str>>()[..])?;

        info!("Removing build packages");
        if args.container == ContainerType::Runtime {
            chroot.apt_remove(&RUNTIME_CLEANUP)?;
        }
    }

    // Name and save out container
    info!("Finalizing and exporting container image.");
    let name = format!("pop-container-{}", args.container);
    let file_name = format!("{}.tar", name);
    Command::new("buildah")
        .args(["commit", "--squash", "--rm", &container, &name])
        .status()?;
    Command::new("podman")
        .args(["save", "-o", &file_name, &name])
        .status()?;
    Command::new("buildah").args(["rmi", &name]).status()?;

    if let Some(user) = username {
        Command::new("chown").args([&user, &file_name]).status()?;
        Command::new("chgrp").args([&user, &file_name]).status()?;
    }

    Ok(())
}

#[derive(Debug)]
enum Errors {
    Io(io::Error),
    Utf(str::Utf8Error),
    BoxedStd(Box<dyn std::error::Error>),
    OsString(OsString),
}

impl From<io::Error> for Errors {
    fn from(io: io::Error) -> Self {
        Errors::Io(io)
    }
}

impl From<str::Utf8Error> for Errors {
    fn from(utf: str::Utf8Error) -> Self {
        Errors::Utf(utf)
    }
}

impl From<Box<dyn std::error::Error>> for Errors {
    fn from(std: Box<dyn std::error::Error>) -> Self {
        Errors::BoxedStd(std)
    }
}
impl From<OsString> for Errors {
    fn from(os_string: OsString) -> Self {
        Errors::OsString(os_string)
    }
}

fn watch(mut command: Command) -> Result<ExitStatus, std::io::Error> {
    let mut cmd = command.stdout(Stdio::piped()).spawn().unwrap();

    {
        let stdout = cmd.stdout.as_mut().unwrap();
        let stdout_reader = BufReader::new(stdout);
        let stdout_lines = stdout_reader.lines();

        for line in stdout_lines {
            info!("{}", line.unwrap_or(String::new()));
        }
    }

    cmd.wait()
}

fn get_username() -> Option<String> {
    const CACHE: &str = "/tmp/pop-container-name";
    if let Some(name) = users::get_current_username() {
        if name != "root" {
            let name = name.into_string().unwrap_or("root".to_string());
            fs::write(CACHE, &name).ok();
            Some(name)
        } else {
            Some(fs::read_to_string(CACHE).unwrap())
        }
    } else {
        Some("root".to_string())
    }
}
