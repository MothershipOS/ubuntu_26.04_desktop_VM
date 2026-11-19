# Ubuntu desktop guest (Ubuntu 26.04 LTS, GNOME)

A stock Ubuntu desktop VM that runs next to PixelOS, on the USB-C external display. VMM: crosvm (as PixelOS) with
the virtio-gpu DRM native context: the guest's Mesa freedreno drives the host Adreno almost directly. Created from
the official arm64 cloud image (cloud-init installs the desktop on the first start).

No mothershipOS program runs in the guest. What mothershipOS adds is in the VM's kernel (`linux/`: every driver
built in, mothershipOS's guest drivers too, no modules, no initrd) and its packages, and the host follows the guest
from outside: its display (crosvm's window), its decoder sessions, its own suspend (PSCI).

## Install (on the phone)
The VM is built on the phone from this repository:

```
mship-system vm-install https://github.com/MothershipOS/ubuntu_26.04_desktop_VM
```

or from a [VM store](https://github.com/MothershipOS/vmstore): `mship-system vm-install <store>/ubuntu-desktop/package.tar.xz`
(this repository's build with the kernel and packages included).

`mothership-build` (mothershipOS runs it, then removes the clone and the build):
- downloads the official `ubuntu-26.04-server-cloudimg-arm64.img` (checked against Ubuntu's SHA256SUMS) and makes
  it a sparse 32 GB raw disk (`qcow2-to-raw`; `MSHIP_VM_DISK` to change);
- takes the VM's kernel and guest packages from `assets/` next to it (the store's package), else from this
  repository's latest release (`MSHIP_ASSETS` for another place: a URL or a directory);
- writes a cloud-init NoCloud seed (`seed/`): user `ubuntu` (password `ubuntu`, sudo without password, the host's
  VM key for ssh), `ubuntu-desktop-minimal`, zram swap, GDM autologin, on-screen keyboard; GNOME keeps its own
  blank (5 min) and suspend (15 min); cloud-init switches itself off at the end (`/etc/cloud/cloud-init.disabled`).

The first start (shell: Start) boots the cloud image, cloud-init installs the desktop from the Ubuntu archive (the
seed is served on the VM's tap, ~15 min, needs the host's internet) together with our Mesa and VA-API packages,
then the guest reboots into GNOME. Later starts boot straight into the desktop. Installing again updates the VM:
the kernel is replaced, the disk (the user's data) and the used seed stay.

## Build the release (on a PC)
In mothershipOS's build image, with mothershipOS checked out next to this repository (its guest drivers):
`cargo xtask release` builds the kernel (`guest-kernel`), the VA-API package (`guest-vaapi`) and Mesa
(`guest-mesa`) into `target/release-assets/`: publish `Image` and `guest-packages.tar` as a release of this
repository, and copy `store/ubuntu-desktop` (the package with both included, `store/info.txt`, `store/icon.svg`) into
the VM store.

## GPU (native context)
Ubuntu's Mesa builds freedreno for the msm kernel driver only. Under crosvm the guest needs freedreno's virtio
interface too: `cargo xtask guest-mesa` cross-compiles Ubuntu's own Mesa package (minutes) with
`freedreno-kmds=msm,virtio` and the drivers a guest uses (freedreno, virgl, softpipe; turnip), version `+mship1`.
This repository's release carries the packages and the VM's first start installs them; for an existing VM,
`dpkg -i` them by hand (and again after an Ubuntu Mesa update). Verified: `eglinfo` renderer FD650,
OpenGL 4.6.

Snaps bring their own Mesa (`gpu-2404`, without the virtio interface) and render on the CPU: Ubuntu's Firefox
snap ran pages at 4K in software (YouTube struggled, the main process at ~170% CPU). Firefox is installed from
Mozilla's APT repository instead (`packages.mozilla.org`, key fingerprint
`35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3`, pinned with priority 1000, `--allow-downgrades` once over Ubuntu's
snap wrapper package), which uses the system Mesa; the dock favourite is `firefox.desktop`.

## Video decode (Venus)
The launcher passes the host's Venus decoder in with `--v4l2-proxy` (virtio-media); the guest kernel has the
virtio-media driver built in: `/dev/video0` is a standard V4L2 stateful decoder (GStreamer, `mpv
--hwdec=v4l2m2m-copy`). Browsers decode through VA-API: the package `cargo xtask guest-vaapi` builds (in this
repository's release; the first start installs it; by hand `sudo apt install ./<deb>`): the
venus VA-API driver over that node, `LIBVA_DRIVER_NAME=venus` for the session, and a Firefox policy (hardware
decode on, AV1 off so YouTube serves VP9, which Venus decodes up to 4K).

## Power (mship.power)
The VM sleeps like stock Ubuntu: GNOME blanks after 5 min idle and suspends after 15 min, and the host follows.
- Blank: the guest's virtio-gpu disables its scanout, crosvm closes its window, the presenter switches the
  monitor off and tells mothershipd (`vm.display`): the VM counts as screen off, may sleep (unless it decodes
  video: crosvm has the decoder open).
- Suspend: the guest kernel's PSCI SYSTEM_SUSPEND (`/sys/power/mem_sleep`: `deep`) comes to crosvm (our patch,
  `host/gpu/patches/crosvm/0004-psci-system-suspend.patch`), which parks the vCPU and asks mothershipd for `vm.sleep`: the VM is frozen,
  its USB devices back on the host. A key, the mouse or Wake (mothershipOS › Power & sleep) resumes it at the
  kernel's resume entry; the guest resets its xHCI and finds its devices again.

## Boot and network under crosvm
- crosvm has no UEFI here: the launcher boots the VM's own kernel (`Image`, vm.json `boot.kernel`, from this
  repository's release) straight onto the disk's first partition. Ubuntu's own kernel packages are not used (and not installed: an
  Ubuntu kernel update would only rebuild initramfs images).
- Network: tap `ubtap0` (host 192.168.98.1, guest 192.168.98.2 by a fixed DHCP lease on its MAC
  52:54:00:12:34:56, which also keeps cloud-init's netplan match), NATed out of the phone's uplink all the time
  (`mship-vm-net`, chain `MSHIP_VMNAT`), independent of the lendable internet slot and of other VMs.
- Stop: `mship vm stop` presses the guest's power button (`crosvm powerbtn`, our crosvm patch
  `0006-aarch64-power-button`; logind powers off), then `crosvm stop` if it is still running after 60 s.

## Run
- Its screen is the USB-C external display: hardware item `display-ext` (Settings › Hardware). The launcher
  starts the VM's own presenter (`mship-presenter`, `MSHIP_VM_OUTPUT=DP-1`; `MSHIP_VM_MODE` picks a mode, else the
  monitor's preferred one) and crosvm draws into it. The guest gets the display when a monitor is connected, at its
  mode, and loses it when unplugged (virtio-gpu display hotplug); while the VM holds `display-ext`, sway leases DP-1 to that presenter.
  mothershipOS never uses DP-1 (`output DP-1 disable`), and Android keeps the panel. Keyboard and mouse: the
  monitor's or dock's USB devices (hardware item `usb`).
- Sound: the external display's audio (DisplayPort over USB-C). The guest has a virtio-snd card (`VirtIO
  SoundCard`, PipeWire's default output) from `vhost-device-sound` (`mothershipOS/host/vm-sound/`, built with our ALSA
  patches), which plays on the phone card's DisplayPort device through a dmix (`<vm dir>/audio/.asoundrc`); a
  silent stream keeps that device open while the VM runs (a DP PCM reopened within ~0.5 s fails in the
  kernel). crosvm needs `max-queue-size=64` for it. `MSHIP_VM_SOUND=0` starts the VM without sound: crosvm stops the
  VM when the sound backend dies.
- Shell: Start / Stop (Show only says it's on the external display).
- ssh: `ssh -i /root/.ssh/id_ed25519_vm ubuntu@192.168.98.2`; console `serial.log` (hvc0).
- CPUs and memory: the VM pool (every CPU, the pool's RAM through a balloon with free page reporting; mothershipd
  moves memory to the VMs that are awake). Caps: vm.json `resources.cpus` / `resources.memory_mb`, or
  `MSHIP_VM_MEM`. The host swap file (installed by `mship-system setup`) keeps a first-start install from running
  the host out of memory while PixelOS runs.
- First start notes: if the first-boot install is interrupted (e.g. the host runs out of memory), the next
  cloud-init run fails with "dpkg was interrupted"; finish it over ssh with `sudo dpkg --configure -a` and
  `sudo apt-get install ubuntu-desktop-minimal`.

## Not yet (platform gaps: [efi-virt-arm64](https://github.com/MothershipOS/mothershipOS/blob/main/docs/platforms/efi-virt-arm64-1.md))
- The platform `efi-virt-arm64` also defines UEFI boot; the launcher only boots the VM's kernel directly.
- The network is its own NAT, not the `internet` slot; no Wi-Fi or Bluetooth handles in this VM yet.
- Sound follows the VM, not the `display-ext` item: the guest's sound card stays while the monitor is unplugged
  or lent elsewhere (it plays into nothing), and there is no capture (DP has no input).
