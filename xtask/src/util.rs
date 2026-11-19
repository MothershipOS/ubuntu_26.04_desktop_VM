//! Helpers of the tasks: paths, downloads, archives, Docker, cross builds.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use sha2::Digest;

/// This repository (ubuntu_26.04_desktop_VM)
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// mothershipOS, for the guest drivers' sources (guest/drivers) and its release: $MSHIP_SOURCE, or the checkout next
/// to this one
pub fn mothership() -> Result<PathBuf> {
    let p = std::env::var_os("MSHIP_SOURCE").map(PathBuf::from).unwrap_or_else(|| root().parent().unwrap().join("mothershipOS"));
    ensure!(p.join("guest/drivers").is_dir(),
            "{}: no mothershipOS (git clone https://github.com/MothershipOS/mothershipOS next to this repository, or set MSHIP_SOURCE)", p.display());
    Ok(p)
}

pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("target"))
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Copy `src` (a file or directory) to `dst`, leaving files whose contents are unchanged alone (their timestamps
/// decide what the kernel build redoes).
pub fn sync(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            sync(&e.path(), &dst.join(e.file_name()))?;
        }
    } else if std::fs::read(dst).ok() != Some(std::fs::read(src)?) {
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(src, dst).with_context(|| dst.display().to_string())?;
    }
    Ok(())
}

/// Add `line` to a file once.
pub fn add_line(file: &Path, line: &str, before_last: Option<&str>) -> Result<()> {
    let text = std::fs::read_to_string(file)?;
    if text.lines().any(|l| l == line) {
        return Ok(());
    }
    let text = match before_last.and_then(|m| text.rfind(m)) {
        Some(i) => format!("{}{line}\n\n{}", &text[..i], &text[i..]),
        None => format!("{text}{line}\n"),
    };
    std::fs::write(file, text)?;
    Ok(())
}

/// `url` saved as `dest` (once), checked against its pinned SHA-256.
pub fn fetch(url: &str, sum: &str, dest: &Path) -> Result<PathBuf> {
    fetch_checked(url, sum, dest, sha256)
}

pub fn sha512(path: &Path) -> Result<String> {
    let mut h = sha2::Sha512::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `url` saved as `dest` (once), checked with `digest` against `sum`.
pub fn fetch_checked(url: &str, sum: &str, dest: &Path, digest: fn(&Path) -> Result<String>) -> Result<PathBuf> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    if !dest.exists() {
        println!("downloading {url}");
        let partial = dest.with_extension("partial");
        let mut body = ureq::get(url).call()?.into_body();
        std::io::copy(&mut body.as_reader(), &mut std::fs::File::create(&partial)?)?;
        std::fs::rename(&partial, dest)?;
    }
    let got = digest(dest)?;
    if got != sum {
        std::fs::remove_file(dest)?;
        bail!("{}: checksum {got}, expected {sum} (removed)", dest.display());
    }
    Ok(dest.to_path_buf())
}

/// One member of a Debian package's ar archive.
pub fn ar_member(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    out.extend(format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, "100644", data.len()).as_bytes());
    out.extend(data);
    if data.len() % 2 == 1 {
        out.push(b'\n');
    }
}

/// A gzip'd tar of `files` ((path in the archive, contents, mode)), with the directories they need.
pub fn tar_gz(files: &[(String, Vec<u8>, u32)]) -> Result<Vec<u8>> {
    let mut t = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best()));
    let mut dirs = std::collections::BTreeSet::new();
    for (path, _, _) in files {
        let mut p = Path::new(path);
        while let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty() && *d != Path::new(".")) {
            dirs.insert(d.to_path_buf());
            p = d;
        }
    }
    for d in &dirs {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o755);
        h.set_size(0);
        h.set_cksum();
        t.append_data(&mut h, format!("{}/", d.display()), std::io::empty())?;
    }
    for (path, data, mode) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(*mode);
        h.set_cksum();
        t.append_data(&mut h, path, data.as_slice())?;
    }
    Ok(t.into_inner()?.finish()?)
}

/// `docker` with these arguments (the host's Docker, from the build image: its socket is mounted).
pub fn docker(args: &[&str]) -> Result<()> {
    ensure!(Command::new("docker").args(args).status().context("docker (mount /var/run/docker.sock)")?.success(), "docker {} failed", args.join(" "));
    Ok(())
}

/// `docker` with these arguments, its output.
pub fn docker_output(args: &[&str]) -> Result<String> {
    let o = Command::new("docker").args(args).output().context("docker (mount /var/run/docker.sock)")?;
    ensure!(o.status.success(), "docker {} failed: {}", args.join(" "), String::from_utf8_lossy(&o.stderr));
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Where the host sees a path under the target directory (for volumes of containers the host's Docker runs).
pub fn host_path(p: &Path) -> Result<String> {
    let host = std::env::var("MSHIP_HOST_TARGET").context("MSHIP_HOST_TARGET: the host path of the target directory")?;
    Ok(Path::new(&host).join(p.strip_prefix(target_dir())?).to_string_lossy().into_owned())
}

/// A meson cross file for the phone (aarch64, the build image's arm64 libraries).
pub fn meson_cross(dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let cross = dir.join("aarch64.ini");
    std::fs::write(
        &cross,
        "[binaries]\nc = 'aarch64-linux-gnu-gcc'\ncpp = 'aarch64-linux-gnu-g++'\nar = 'aarch64-linux-gnu-ar'\nstrip = 'aarch64-linux-gnu-strip'\n\
         pkg-config = 'pkg-config'\n[properties]\npkg_config_libdir = ['/usr/lib/aarch64-linux-gnu/pkgconfig', '/usr/share/pkgconfig']\n\
         [host_machine]\nsystem = 'linux'\ncpu_family = 'aarch64'\ncpu = 'aarch64'\nendian = 'little'\n",
    )?;
    Ok(cross)
}

/// Run a program in `dir`; an error names it.
pub fn run_in(dir: &Path, args: &[&str]) -> Result<()> {
    let st = Command::new(args[0]).args(&args[1..]).current_dir(dir).status().with_context(|| args[0].to_string())?;
    ensure!(st.success(), "{} failed (in {})", args.join(" "), dir.display());
    Ok(())
}

/// The patches of a directory (`*.patch`, in name order).
pub fn patches(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<std::io::Result<_>>()?;
    v.retain(|p| p.extension().is_some_and(|x| x == "patch"));
    v.sort();
    Ok(v)
}

/// Apply patches to a tree (`git apply`, which also works outside a repository).
pub fn apply(tree: &Path, patches: &[PathBuf]) -> Result<()> {
    for p in patches {
        let st = Command::new("git").arg("apply").arg(p).current_dir(tree).status()?;
        ensure!(st.success(), "{}: does not apply", p.display());
    }
    Ok(())
}

/// The build's parallelism.
pub fn jobs() -> String {
    std::thread::available_parallelism().map_or(4, |n| n.get()).to_string()
}

/// A Linux source tree: the pinned tarball (`src`: version, url, sha256) downloaded once, unpacked, with
/// `patches_dir` applied in name order; a changed patch set unpacks it again.
pub fn kernel_tree(work: &Path, src: &serde_json::Value, patches_dir: &Path) -> Result<PathBuf> {
    let (version, url, sum) = (src["version"].as_str().context("version")?, src["url"].as_str().context("url")?, src["sha256"].as_str().context("sha256")?);
    let patches = patches(patches_dir)?;
    let mut h = sha2::Sha256::new();
    for p in &patches {
        h.update(std::fs::read(p)?);
    }
    let stamp: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let tree = work.join(format!("linux-{version}"));
    let applied = tree.join(".mothership-patches");
    if tree.join("Makefile").exists() && std::fs::read_to_string(&applied).is_ok_and(|s| s == stamp) {
        return Ok(tree);
    }
    let _ = std::fs::remove_dir_all(&tree);
    let archive = fetch(url, sum, &work.join(format!("linux-{version}.tar.xz")))?;
    println!("unpacking {}", archive.display());
    let staging = work.join("unpack");
    let _ = std::fs::remove_dir_all(&staging);
    tar::Archive::new(xz2::read::XzDecoder::new(std::fs::File::open(&archive)?)).unpack(&staging)?;
    std::fs::rename(staging.join(format!("linux-{version}")), &tree)?;
    std::fs::remove_dir_all(&staging)?;
    apply(&tree, &patches)?;
    std::fs::write(&applied, stamp)?;
    Ok(tree)
}


/// store/info.txt for the store, with the package's version (mothership-package.json "version", the one source)
pub fn store_info(to: &Path) -> Result<()> {
    let pkg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root().join("mothership-package.json"))?)?;
    let version = pkg["version"].as_str().context("mothership-package.json: no \"version\"")?;
    let info = std::fs::read_to_string(root().join("store/info.txt"))?;
    let (head, desc) = info.split_once("\n\n").unwrap_or((info.as_str(), ""));
    let head: Vec<&str> = head.lines().filter(|l| !l.starts_with("version:")).collect();
    std::fs::write(to, format!("{}\nversion: {version}\n\n{desc}", head.join("\n")))?;
    Ok(())
}
