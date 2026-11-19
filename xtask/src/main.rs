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
//!   target/release-assets/guest-packages.tar; and this VM's folder for a VM store (vmstore),
//!   target/release-assets/store/ubuntu-desktop: package.tar.xz (mothership-build and its files, with both assets
//!   in assets/, so the store's package needs no release), info.txt and the icon (store/)

use anyhow::{Result, bail, ensure};

mod guest;
mod util;

use guest::*;
use util::*;

fn release() -> Result<()> {
    // its own files only: the folder is shared with other VMs' releases
    let out = target_dir().join("release-assets");
    let _ = std::fs::remove_dir_all(out.join("packages"));
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
    store(&out)?;
    println!("{}: Image, guest-packages.tar ({} packages): publish both as assets of a release of this repository; \
              store/: copy it into the VM store", out.display(), debs.len());
    Ok(())
}

/// the files mothership-build needs, in the store's package
const BUILD_FILES: &[&str] = &["mothership-build", "qcow2-to-raw", "user-data.in", "vm.json", "mothership-package.json"];

/// store/ubuntu-desktop: the package (the build with its assets), info.txt, the icon
fn store(assets: &std::path::Path) -> Result<()> {
    let dir = assets.join("store/ubuntu-desktop");
    let stage = assets.join("stage-ubuntu");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(stage.join("assets"))?;
    for f in BUILD_FILES {
        std::fs::copy(root().join(f), stage.join(f))?;
    }
    for f in ["Image", "guest-packages.tar"] {
        std::fs::copy(assets.join(f), stage.join("assets").join(f))?;
    }
    for f in std::fs::read_dir(root().join("store"))? {
        let f = f?;
        std::fs::copy(f.path(), dir.join(f.file_name()))?;
    }
    store_info(&dir.join("info.txt"))?;
    let archive = dir.join("package.tar.xz");
    let st = std::process::Command::new("tar").env("XZ_OPT", "-T0 -6").arg("-cJf").arg(&archive).arg("-C").arg(&stage).arg(".").status()?;
    ensure!(st.success(), "tar {}", archive.display());
    std::fs::remove_dir_all(&stage)?;
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
