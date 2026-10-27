//! The Linux guests: their kernel, VA-API driver package and Mesa packages.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::util::*;

/// The guests' kernel source tree (linux/: the pinned source, linux/patches applied).
fn kernel_source(work: &Path) -> Result<PathBuf> {
    let src: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root().join("linux/source.json"))?)?;
    kernel_tree(work, &src, &root().join("linux/patches"))
}

/// The Linux guests' kernel: every driver built in (mothershipOS's guest/drivers in drivers/mothership).
pub fn guest_kernel() -> Result<PathBuf> {
    let work = target_dir().join("guest-kernel");
    let tree = kernel_source(&work)?;
    let ours = tree.join("drivers/mothership");
    for f in ["Kconfig", "Makefile", "virtio_media", "mship_wifi"] {
        sync(&mothership()?.join("guest/drivers").join(f), &ours.join(f))?;
    }
    add_line(&tree.join("drivers/Kconfig"), "source \"drivers/mothership/Kconfig\"", Some("endmenu"))?;
    add_line(&tree.join("drivers/Makefile"), "obj-y\t\t\t\t+= mothership/", None)?;
    sync(&root().join("linux/defconfig"), &tree.join("arch/arm64/configs/mothership_guest_defconfig"))?;
    let out = work.join("build");
    let jobs = jobs();
    for goal in ["mothership_guest_defconfig", "Image"] {
        let st = Command::new("make")
            .args(["-s", "-C"])
            .arg(&tree)
            .arg(format!("O={}", out.display()))
            .args(["ARCH=arm64", "CROSS_COMPILE=aarch64-linux-gnu-", "-j", &jobs, goal])
            .status()?;
        ensure!(st.success(), "kernel: make {goal} failed");
    }
    let image = out.join("arch/arm64/boot/Image");
    println!("{}", image.display());
    Ok(image)
}

/// The Venus VA-API driver for Linux guests (upstream at a pinned commit + our patch, mothershipOS's guest/drivers/venus-vaapi),
/// cross-built and packaged with its session environment and Firefox policy:
/// target/guest-vaapi/mothership-venus-vaapi_<version>_arm64.deb
pub fn guest_vaapi() -> Result<PathBuf> {
    let dir = mothership()?.join("guest/drivers/venus-vaapi");
    let src: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("source.json"))?)?;
    let (version, rev, url, sum) = (
        src["version"].as_str().context("version")?,
        src["rev"].as_str().context("rev")?,
        src["url"].as_str().context("url")?,
        src["sha256"].as_str().context("sha256")?,
    );
    let work = target_dir().join("guest-vaapi");
    let archive = fetch(url, sum, &work.join(format!("venus-vaapi-driver-{rev}.tar.gz")))?;
    // a fresh tree every build: the patch applies to the pristine source
    let tree = work.join(format!("venus-vaapi-driver-{rev}"));
    let _ = std::fs::remove_dir_all(&tree);
    tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(&archive)?)).unpack(&work)?;
    let patch = dir.join("0001-mothership-venus-vaapi-fixes.patch");
    ensure!(Command::new("git").arg("apply").arg(&patch).current_dir(&tree).status()?.success(), "{}: does not apply", patch.display());
    let cross = meson_cross(&work)?;
    let build = tree.join("build");
    let run = |args: &[&str]| -> Result<()> {
        ensure!(Command::new(args[0]).args(&args[1..]).current_dir(&tree).status()?.success(), "{} failed", args.join(" "));
        Ok(())
    };
    run(&["meson", "setup", "--cross-file", &cross.to_string_lossy(), "--prefix=/usr", "--libdir=lib/aarch64-linux-gnu",
          "--buildtype=release", "-Dwerror=false", "build"])?;
    run(&["ninja", "-C", "build"])?;
    let so = build.join("venus_drv_video.so");
    ensure!(Command::new("aarch64-linux-gnu-strip").arg(&so).status()?.success(), "strip failed");

    let deb_version = format!("{version}+mothership{}", std::fs::read_to_string(mothership()?.join("VERSION"))?.trim());
    let data = tar_gz(&[
        ("./usr/lib/aarch64-linux-gnu/dri/venus_drv_video.so".into(), std::fs::read(&so)?, 0o644),
        ("./etc/environment.d/80-mothership-vaapi.conf".into(), std::fs::read(dir.join("environment.conf"))?, 0o644),
        ("./etc/firefox/policies/policies.json".into(), std::fs::read(dir.join("firefox-policies.json"))?, 0o644),
    ])?;
    let control = format!(
        "Package: mothership-venus-vaapi\nVersion: {deb_version}\nArchitecture: arm64\nMaintainer: mothershipOS\n\
         Depends: libva2, libdrm2, libgbm1\nSection: video\nPriority: optional\n\
         Description: VA-API hardware video decoding on the mothershipOS phone's Venus decoder\n \
         The venus VA-API driver (upstream {rev}, with mothershipOS's fixes) for the Venus\n \
         decoder the host lends through virtio-media, the session environment that selects it,\n \
         and a Firefox policy that turns hardware decoding on.\n"
    );
    let control = tar_gz(&[("./control".into(), control.into_bytes(), 0o644)])?;
    let mut deb = b"!<arch>\n".to_vec();
    ar_member(&mut deb, "debian-binary", b"2.0\n");
    ar_member(&mut deb, "control.tar.gz", &control);
    ar_member(&mut deb, "data.tar.gz", &data);
    let path = work.join(format!("mothership-venus-vaapi_{deb_version}_arm64.deb"));
    std::fs::write(&path, deb)?;
    println!("{}", path.display());
    Ok(path)
}

/// Mesa packages the guests replace: Ubuntu's, with freedreno's virtio kernel interface too
const GUEST_MESA_DEBS: &[&str] =
    &["libegl-mesa0", "libgbm1", "libgl1-mesa-dri", "libglx-mesa0", "mesa-libgallium", "mesa-vulkan-drivers"];

/// Ubuntu's own Mesa package for the Linux guests, rebuilt with freedreno over virtio-gpu (DRM native context) as
/// well as msm: Ubuntu builds freedreno for the msm kernel driver only, and under crosvm the guest then renders on
/// the CPU. Cross-compiled for arm64 (minutes) with the drivers a guest uses, so without LLVM (whose packages
/// cannot be cross-installed): gallium freedreno, virgl, softpipe; Vulkan freedreno (+ Ubuntu's general layers). Version `<Ubuntu's>+mship1`.
pub fn guest_mesa() -> Result<Vec<PathBuf>> {
    const IMAGE: &str = "mothership-guest-mesa:26.04";
    docker(&["build", "-q", "-t", IMAGE, &root().join("xtask/docker/guest-mesa").to_string_lossy()])?;
    let work = target_dir().join("guest-mesa");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    // the archive's current source package: apt resolves it in the image, the files are fetched and unpacked here
    let uris = docker_output(&["run", "--rm", IMAGE, "apt-get", "source", "--print-uris", "-qq", "mesa"])?;
    for l in uris.lines() {
        // 'URL' filename size SHA512:hex (or SHA256:)
        let w: Vec<&str> = l.split_whitespace().collect();
        let (Some(url), Some(name), Some(hash)) = (w.first(), w.get(1), w.get(3)) else { continue };
        let url = url.trim_matches('\'');
        match hash.split_once(':') {
            Some(("SHA512", sum)) => fetch_checked(url, sum, &work.join(name), sha512)?,
            Some(("SHA256", sum)) => fetch(url, sum, &work.join(name))?,
            _ => bail!("mesa source {name}: no SHA-256/512 ({hash})"),
        };
    }
    let dsc = std::fs::read_dir(&work)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "dsc"))
        .context("apt-get source mesa: no .dsc")?;
    run_in(&work, &["dpkg-source", "--no-check", "-x", &dsc.file_name().unwrap().to_string_lossy()])?;
    let src = std::fs::read_dir(&work)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mesa-")))
        .context("apt-get source mesa: no source tree")?;
    // no LLVM, rusticl, NVK or valgrind (the arch lists match nothing: their packages and build dependencies drop
    // out of debian/control too), our driver set, freedreno's kernel interfaces msm and virtio
    let rules = src.join("debian/rules");
    let mut text = std::fs::read_to_string(&rules)?;
    for var in ["LLVM_ARCHS", "RUSTICL_ARCHS", "NVK_ARCHS", "VALGRIND_ARCHS"] {
        let line = text.lines().find(|l| l.starts_with(&format!("{var} "))).with_context(|| format!("debian/rules: no {var}"))?.to_string();
        text = text.replace(&line, &format!("{var} = none"));
    }
    let anchor = "\nempty:=\n";
    ensure!(text.matches(anchor).count() == 1, "debian/rules: no single `empty:=`");
    text = text.replace(anchor, "\n# mothershipOS: the guest's drivers; freedreno over virtio-gpu DRM native context too (crosvm, the host Adreno)\n\
        GALLIUM_DRIVERS = softpipe freedreno virgl\nVULKAN_DRIVERS = freedreno\nVULKAN_LAYERS = device-select overlay screenshot\n\
        confflags_GALLIUM += -Dfreedreno-kmds=msm,virtio\n\nempty:=\n");
    std::fs::write(&rules, text)?;
    // the Vulkan package is listed for the LLVM architectures (for lavapipe): ours has turnip, no RADV
    let control = src.join("debian/control.in");
    let text = std::fs::read_to_string(&control)?;
    let stanza = "Package: mesa-vulkan-drivers\nSection: libs\nArchitecture: @LLVM_ARCHS@\n";
    ensure!(text.contains(stanza), "debian/control.in: mesa-vulkan-drivers is not for @LLVM_ARCHS@");
    std::fs::write(&control, text.replace(stanza, "Package: mesa-vulkan-drivers\nSection: libs\nArchitecture: arm64\n"))?;
    let install = src.join("debian/mesa-vulkan-drivers.install");
    let text = std::fs::read_to_string(&install)?;
    std::fs::write(&install, text.lines().filter(|l| !l.contains("radv")).map(|l| format!("{l}\n")).collect::<String>())?;
    // a local version above Ubuntu's: <version>+mship1
    let changelog = src.join("debian/changelog");
    let text = std::fs::read_to_string(&changelog)?;
    let first = text.lines().next().context("debian/changelog")?;
    let (version, dist) = (
        first.split(['(', ')']).nth(1).context("changelog version")?,
        first.split(')').nth(1).and_then(|r| r.trim().split(';').next()).context("changelog distribution")?.trim(),
    );
    let date = String::from_utf8(Command::new("date").arg("-R").output()?.stdout)?;
    std::fs::write(&changelog, format!(
        "mesa ({version}+mship1) {dist}; urgency=medium\n\n  * mothershipOS: freedreno with the msm and virtio (DRM native context) kernel interfaces; no LLVM.\n\n -- mothershipOS <mship@localhost>  {}\n\n{text}",
        date.trim()
    ))?;
    let dir = src.file_name().unwrap().to_string_lossy().into_owned();
    println!("cross-building mesa {version}+mship1 for arm64");
    // the build dependencies: libraries for arm64, tools (and Architecture: all packages) native; Mesa's control
    // file lacks the :native marks that would let apt cross-resolve them itself. The Vulkan layers' libpng and
    // libvulkan are listed for the LLVM architectures only.
    let script = format!(
        "set -e; trap 'chown -R {uid}:{gid} /w' EXIT; cd /w/{dir}; debian/rules regen_control >/dev/null; \
         unmet=$(dpkg-checkbuilddeps -a arm64 2>&1 | sed -n 's/.*unmet build dependencies: //p'); pk='libpng-dev:arm64 libvulkan-dev:arm64'; \
         for d in $(echo \"$unmet\" | sed 's/([^)]*)//g'); do case $d in *:native) continue;; esac; \
           if dpkg -s \"$d\" >/dev/null 2>&1 && ! apt-cache show \"$d\" | grep -qm1 '^Multi-Arch: same'; then continue; fi; \
           if apt-cache show \"$d\" | grep -qm1 '^Architecture: all'; then pk=\"$pk $d\"; else pk=\"$pk $d:arm64\"; fi; done; \
         apt-get update -qq; apt-get install -y -qq --no-install-recommends $pk >/dev/null; \
         DEB_BUILD_OPTIONS='parallel={jobs} nocheck noautodbgsym' dpkg-buildpackage -a arm64 -B -d -uc -us >&2",
        jobs = jobs(), uid = unsafe { libc::getuid() }, gid = unsafe { libc::getgid() }
    );
    let st = Command::new("docker")
        .args(["run", "--rm", "-v", &format!("{}:/w", host_path(&work)?), IMAGE, "sh", "-c", &script])
        .status()?;
    ensure!(st.success(), "mesa: dpkg-buildpackage failed");
    let debs = guest_mesa_built();
    ensure!(debs.len() >= GUEST_MESA_DEBS.len(), "mesa: expected the packages {GUEST_MESA_DEBS:?}, built {debs:?}");
    for d in &debs {
        println!("{}", d.display());
    }
    Ok(debs)
}

/// The last guest-mesa build's packages (none if it never ran).
pub fn guest_mesa_built() -> Vec<PathBuf> {
    std::fs::read_dir(target_dir().join("guest-mesa"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.ends_with("+mship1_arm64.deb") && (GUEST_MESA_DEBS.iter().any(|d| n.starts_with(&format!("{d}_"))) || n.starts_with("libgallium"))
        })
        .collect()
}
