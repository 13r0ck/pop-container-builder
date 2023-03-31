use std::process::Command;
use std::{str, io};

use distinst::steps::configure::ChrootConfigurator;
use distinst::chroot::Chroot;
use sudo;
//use cascade::cascade;

fn main() -> Result<(), Errors> {

  sudo::escalate_if_needed()?;

  let output = Command::new("buildah").args(["from", "scratch"]).output()?;
  let len = output.stdout.len() - 1;
  let container = str::from_utf8(&output.stdout[..len])?;

  let output = Command::new("buildah").args(["mount", &container]).output()?;
  let len = output.stdout.len() - 1;
  let mount_point = str::from_utf8(&output.stdout[..len])?;

  let output = Command::new("debootstrap").args(["--variant=minbase", "stable", mount_point, "http://deb.debian.org/debian"]).output();
  println!("{:?}", output);
  
  // Install into container
  {
  let chroot =
      Chroot::new(&mount_point)?;
      //..clear_envs(true);
      //..env("DEBIAN_FRONTEND", "noninteractive");
      //..env("HOME", "/root");
      // TODO make "en_US" work for other languages
      //..env("LC_ALL", "en_US");
      //..env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
  let chroot = ChrootConfigurator::new(chroot);
  chroot.apt_install(&["hello"]);
  }
  /*
  println!("chroot setup done");
  */



  let output = Command::new("buildah").args(["commit", "--squash", "--rm", &container, "pop-container"]).output()?;
  let len = output.stdout.len() - 1;
  println!("{:?}", str::from_utf8(&output.stdout[..len])?);

    Ok(())
}

#[derive(Debug)]
enum Errors {
  Io(io::Error),
  Utf(str::Utf8Error),
  BoxedStd(Box<dyn std::error::Error>),
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
