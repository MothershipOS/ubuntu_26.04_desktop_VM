//! The Ubuntu desktop VM's build tasks (`cargo xtask <task>`), on a PC: what the VM gets from us besides Ubuntu's own
//! image. The VM itself is built on the phone by `mothership-build` (mothershipOS runs it: `mship-system vm-install`),
//! which downloads these from this repository's latest release. Run them where mothershipOS's own tasks run (its
//! build image: Rust, the phone's cross toolchain, Docker); the guest drivers come from mothershipOS (`guest/drivers`,
//! the checkout next to this one or MSHIP_SOURCE).
//!
//! - `guest-kernel`: the VM's kernel (linux/: the pinned source, its patches and config, mothershipOS's guest drivers
//!   built in; no modules, no initrd), target/guest-kernel/build/arch/arm64/boot/Image
//! - `guest-vaapi`: the VA-API driver package (mothershipOS's guest/drivers/venus-vaapi),
//!   target/guest-vaapi/mothership-venus-vaapi_<version>_arm64.deb
//! - `guest-mesa`: Ubuntu's Mesa with freedreno's virtio interface, target/guest-mesa (cross-compiled through Docker)
//! - `release`: all three, as the release assets mothership-build downloads: target/release-assets/Image and
//!   target/release-assets/guest-packages.tar

use anyhow::{Result, bail, ensure};

mod guest;
mod util;

use guest::*;
use util::*;

fn release() -> Result<()> {
    let out = target_dir().join("release-assets");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("packages"))?;
    std::fs::copy(guest_kernel()?, out.join("Image"))?;
    let mut debs = vec![guest_vaapi()?];
    debs.extend(guest_mesa()?);
    for d in &debs {
        std::fs::copy(d, out.join("packages").join(d.file_name().unwrap()))?;
    }
    let tar = out.join("guest-packages.tar");
    let st = std::process::Command::new("tar").arg("-cf").arg(&tar).arg("-C").arg(out.join("packages")).arg(".").status()?;
    ensure!(st.success(), "tar {}", tar.display());
    std::fs::remove_dir_all(out.join("packages"))?;
    println!("{}: Image, guest-packages.tar ({} packages): publish both as assets of a release of this repository", out.display(), debs.len());
    Ok(())
}

fn main() -> Result<()> {
    match std::env::args().nth(1).unwrap_or_default().as_str() {
        "guest-kernel" => guest_kernel().map(|_| ()),
        "guest-vaapi" => guest_vaapi().map(|_| ()),
        "guest-mesa" => guest_mesa().map(|_| ()),
        "release" => release(),
        "" | "help" => {
            println!("cargo xtask guest-kernel|guest-vaapi|guest-mesa|release (see xtask/src/main.rs)");
            Ok(())
        }
        other => bail!("unknown task {other:?} (cargo xtask help)"),
    }
}
