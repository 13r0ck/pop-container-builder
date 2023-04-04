mod packages;

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use std::{fs, str};

use distinst::chroot::Chroot;
use distinst::steps::configure::ChrootConfigurator;
use log::info;
use sudo;

use packages::{RUNTIME, RUNTIME_CLEANUP};

const CODENAME: &str = "jammy";
const POPKEY: &str = "204DD8AEC33A7AFF";
const POPKEY_PATHS: [&str; 2] = [
    "/etc/apt/trusted.gpg.d/pop-keyring-2017-archive.gpg",
    "/usr/share/keyrings/pop-archive-keyring.gpg",
];

fn main() -> Result<(), Errors> {
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
    info!("{username:?}");

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
        chroot.apt_add_repository(&[
            &("http://apt.pop-os.org/proprietary", CODENAME, &["main"]),
            &("http://apt.pop-os.org/release", CODENAME, &["main"]),
            &(
                "http://apt.pop-os.org/ubuntu",
                CODENAME,
                &["main", "universe", "multiverse", "restricted"],
            ),
            &(
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-security"),
                &["main", "universe", "multiverse", "restricted"],
            ),
            &(
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-updates"),
                &["main", "universe", "multiverse", "restricted"],
            ),
            &(
                "http://apt.pop-os.org/ubuntu",
                &format!("{CODENAME}-backports"),
                &["main", "universe", "multiverse", "restricted"],
            ),
        ])?;
        info!("Installing Updates.");
        chroot.apt_update()?;
        chroot.apt_upgrade()?;
        chroot.apt_install(&RUNTIME)?;
        info!("Removing build packages");
        chroot.apt_remove(&RUNTIME_CLEANUP)?;
    }

    info!("Finalizing and exporting container image.");
    Command::new("buildah")
        .args(["commit", "--squash", "--rm", &container, "pop-container"])
        .status()?;
    Command::new("podman")
        .args(["save", "-o", "pop-container-runtime.tar", "pop-container"])
        .status()?;
    Command::new("buildah")
        .args(["rmi", "pop-container"])
        .status()?;

    if let Some(user) = username {
        Command::new("chown")
            .args([&user, "./pop-container-runtime.tar"])
            .status()?;
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
            println!("{}", line.unwrap_or(String::new()));
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
