# nrzube — minimal OS image for the RPi5 (Kubernetes worker, containerd + kubelet)

> **This document is the reference plan and its living status page.**
> The original plan was written in French during the planning session; it is
> kept here in English, with the same technical content, plus a status
> section (§0) that tracks what is done, what remains and the open doubts.

## 0. Status

**Last update:** 2026-11-04 — constrained-environment decisions (environment is
**not airgapped**, but constrained and controlled): NTP via chrony (§5.7),
boot provisioning retry + **full reboot after 10 min of failures** (§5.8),
DHCP option 224 → HTTP config server kept (§10).

### 0.1 What exists in the repository

| Area | State | Where |
|---|---|---|
| Buildroot external tree | done | `buildroot/` (`external.desc`, `external.mk`, `Config.in`) |
| Reference defconfigs | done | `buildroot/configs/nrzube_pi5_defconfig` (full), `nrzube_pi5_boot_defconfig` (boot gate) |
| Kernel fragment | done | `buildroot/board/nrzube/kernel-nrzube.fragment` |
| U-Boot fragment + boot script | done | `buildroot/board/nrzube/uboot-nrzube.fragment`, `boot.cmd` |
| Firmware `config.txt` | done | `buildroot/board/nrzube/config.txt` |
| Rootfs overlay (`/init`, s6 services, sysctl policy, dhcpcd, containerd) | done, **never assembled** | `buildroot/board/nrzube/rootfs-overlay/` |
| chrony/NTP wiring (§5.7) | done, unit tested, never run on hardware | `etc/chrony.conf`, `usr/lib/dhcpcd/60-nrzube-ntp`, `usr/share/nrzube/s6/{chronyd,time-sync}/run`, kubelet time gate, `ntpServers` in the agent |
| Real initramfs (slot A/B, failed-boot counter, rollback, `/etc` overlay) | done, **never assembled** | `buildroot/board/nrzube/initramfs/init` |
| Stage 1 initramfs (boot gate) | done, **built and verified** | `buildroot/board/nrzube/initramfs/stage1-init` |
| Rust agent | done, unit tested, **never run on hardware** | `agent/` |
| Node name / hostname | done | the agent applies `kubeletFlags.nodeName` to the system hostname, `/etc/hostname` and the kubelet `--hostname-override` |
| Image assembly (GPT, FAT, squashfs slots, ext4 state) | done, **only the boot gate variant has been built** | `scripts/build-image.sh` |
| Build drivers (native + container) | done | `scripts/build-all.sh`, `scripts/native-build.sh`, `scripts/local-build.sh` |
| Version pinning | done | `versions.lock`, `versions.mk`, `scripts/refresh-versions-lock.sh` |
| Renovate | done | `renovate.json` |
| CI workflow | **not done** | `.github/workflows/release.yml` (missing) |
| Docs | partial | `docs/gates.md` done; `architecture.md`, `boot-chain.md`, `ota.md`, `troubleshooting.md` missing |

### 0.2 The boot gate artifact (built, verified statically, NOT validated on hardware)

```
dist/nrzube-boot-gate.img          512 MiB, GPT, one FAT32 partition
b3b3b5cf95bf4850fddcf3103b1045e04ceda124af75f423ad534e5f37b52847
```

Contents of the boot partition (verified with `mdir`):

| File | Role |
|---|---|
| `config.txt` | `arm_64bit=1` + `kernel=u-boot.bin` (chainload), `enable_uart=1`, `disable_overscan=1`, `avoid_warnings=2` |
| `start4.elf`, `fixup4.dat` | **RPi GPU firmware** — without them the EEPROM bootloader cannot start the firmware and the board stays dead |
| `bcm2712-rpi-5-b.dtb` | device tree blob of the firmware release, handed over to U-Boot through `x0` |
| `bcm2712-rpi-5-b-kernel.dtb` | DTB compiled from the kernel tree (fallback, only used if the firmware provided none) |
| `u-boot.bin` | U-Boot 2026.10 (`rpi_arm64_defconfig` + nrzube fragment) |
| `boot.scr` | U-Boot script: loads `Image` + `initrd.img`, `booti` with the firmware DTB |
| `Image` | kernel `raspberrypi/linux` `stable_20260724` = **6.18.39**, commit `60ea684a…` verified at build time |
| `initrd.img` | busybox + stage 1 `/init` (mounts `/proc`, `/sys`, `/dev`, prints the banner, drops to a shell) |

Kernel fragment applied and verified in `.config`: `SQUASHFS`, `OVERLAY_FS`,
`IPV6`, `NF_TABLES`, `NF_CONNTRACK`, `BRIDGE`, `VETH`, `BRIDGE_NETFILTER`,
`CGROUPS`, `SECCOMP`, `DRM`, `DRM_VC4`, `FRAMEBUFFER_CONSOLE` are all `=y`
(no module needed in the boot path, and the HDMI console works from the
initramfs).

### 0.3 What remains

| # | Item | Blocking? | Notes |
|---|---|---|---|
| 1 | **Validate the boot gate on a real Pi 5** | yes | the only unknown left in the boot chain |
| 2 | Build the **full image** (p2/p3 squashfs slots, p4 state, rootfs with s6/containerd/kubelet/agent) | yes | all the pieces are written, nothing has ever been assembled or run |
| 3 | Run the real initramfs on hardware (slot selection, overlay, rollback) | yes | gate 2 |
| 4 | Provision a real node (DHCP option 224 → node config → kubelet join) | yes | needs the answers to §10 |
| 5 | CI workflow (build, SHA-256, `cosign sign-blob`, `oras push` to GHCR) | no | `oras`/`cosign` not wired yet |
| 6 | Remaining docs (`architecture.md`, `boot-chain.md`, `ota.md`, `troubleshooting.md`) | no | |
| 7 | Smoke-test script of the full image (boot → slot switch → rollback) | no | |
| 8 | Gate 3 (WAN down), gate 4 (power cut), gate 5 (OTA) | for "operational" | described in `docs/gates.md` |
| 9 | chrony/NTP wiring + kubelet time-sync gate | yes for Phase 4 | **done 2026-11-04** (§5.7): defconfig, overlay, hook, agent `ntpServers`, cargo tests pass |
| 10 | Boot provisioning deadline: full reboot after 10 min of DHCP/HTTP failures (unprovisioned node only) + unit tests | yes for Phase 4 | design in §5.8, not implemented |

### 0.4 Doubts and open risks

**Hardware / boot chain**

1. **GPT vs MBR**: the official RPi images use MBR; the firmware is expected to
   find the boot partition by content (`config.txt`). *Unvalidated* — this is
   the first thing the boot gate will tell us.
2. **EEPROM version**: the boot gate does **not** update the Pi EEPROM (the
   pinned `rpi-eeprom v2026.09.25-2712` is not flashed). An old EEPROM may
   refuse the boot (minimum bootloader version check).
3. **DTB / kernel skew**: the DTB comes from the firmware release
   `1.20260915` while the kernel is `6.18.39`. A skew would show up as a
   failing probe. The kernel-built DTB is shipped as a fallback but U-Boot
   only uses it when the firmware provided no DTB.
4. **`console=tty1`**: the HDMI console needs `CONFIG_DRM_VC4=y`, which in turn
   needs `CONFIG_SND`/`CONFIG_SND_SOC` (the vc4 driver selects the HDMI audio
   helpers). The serial console stays the last one listed, so
   `/dev/console` (and the interactive shell) remains on the UART.

**Build system**

5. **Buildroot 2026.08 + custom git kernel**: the kernel headers series is
   not declared, so the glibc option of the internal toolchain becomes
   invisible and **uClibc is silently selected**. Mitigation: the external
   Bootlin glibc toolchain (see §12.1).
6. **Buildroot 2026.08 + external toolchain**: the staging sysroot is
   populated *after* the `lib` symlinks are created, while the compiler
   wrapper already points to it → the install fails. Worked around in
   `buildroot/external.mk`.
7. **`BR2_DOWNLOAD_FORCE_CHECK_HASHES=y`** cannot be used: Buildroot then
   also demands a digest for the archive it generates itself from the kernel
   git repository. The hash check of the release artifacts is done by our own
   scripts instead (SHA-256 from `versions.lock`).
8. **Nix hosts**: the "format" hardening flag (`-Werror=format-security`)
   breaks the build of GCC's own libcpp, and the Nix loader does not find the
   system `libstdc++`. Both are handled in `scripts/build-all.sh`.

**Product**

9. The environment is **constrained and controlled, not airgapped** (decision
   of 2026-11-04): the LAN provides DHCP (option 224), an HTTP config server,
   NTP servers and the cluster API; GHCR remains reachable for OTA. The open
   questions of §10 are updated accordingly (option 224 kept; the LAN NTP
   addresses are still to be provided).
10. `kube-proxy` and `crictl` were removed from the image (decision of
    2026-11-03): the cluster provides kube-proxy, and without SSH access
    `crictl` has no user. `kubectl` is kept for console debugging.

### 0.5 Deviations from the original plan

| # | Plan said | Reality | Reason |
|---|---|---|---|
| 1 | "glibc compiled by Buildroot" | external Bootlin glibc toolchain | Buildroot cannot select glibc with a custom git kernel (uClibc is silently chosen) |
| 2 | "the RPi5 firmware does not use `start*.elf`/`fixup*.dat` (EEPROM self-contained)" | **wrong**: the EEPROM holds the bootloader, the GPU firmware is loaded from the boot partition | found while building the boot gate: the image would not have booted |
| 3 | `use_tempaddr=0` to stabilise the IPv6 address | `slaac private` only | `use_tempaddr` does not exist in dhcpcd 10.2.4; temporaries are only created with `slaac … temporary` |
| 4 | `board/raspberrypi/linux-4k-page-size.fragment` in the kernel tree | it is a **Buildroot** file (`board/raspberrypi/`) | the kernel tree has no such file |
| 5 | `oci-client` crate for the registry | minimal OCI client written in the agent | full control over the auth flow, no unknown dependency |
| 6 | crictl and kube-proxy in the image | removed | decision of 2026-11-03 |
| 7 | `CONFIG_CGROUP_V2=y` in the fragment | `CONFIG_CGROUPS=y` + the controllers | there is no `CONFIG_CGROUP_V2` symbol: the unified hierarchy comes with `CONFIG_CGROUPS` |
| 8 | "airgapped target" (working assumption during the architecture review of 2026-11-04) | constrained and controlled LAN | decision of 2026-11-04: DHCP option 224 + HTTP config server, LAN NTP and GHCR are available; WAN-down resilience is kept as a robustness requirement (§6), not as the primary mode |

## 1. Objective

Minimal disk image for the Raspberry Pi 5, booting directly on the hardware,
that:

- starts a **DHCP + IPv6 SLAAC (RA)** client towards the existing router;
- runs **containerd + kubelet** and joins an **existing** Kubernetes control
  plane (no kubeadm);
- exposes a **Rust agent** that (a) watches the health of containerd/kubelet,
  (b) applies **signed OTA updates** (cosign) from a trusted OCI registry,
  with automatic **A/B rollback**;
- runs an **NTP client (chrony)** towards the LAN NTP servers — the Pi 5 has
  no battery-backed RTC, so after a power cut the clock is wrong and TLS to
  the API server would fail (§5.7);
- **resists DHCP/HTTP unavailability during boot**: retries the provisioning,
  then performs a **full reboot after 10 minutes of failures** when the node
  is not yet provisioned (§5.8);
- is **built and signed in CI** (cosign), every artifact being **hashed
  (SHA-256)**.

## 2. Decisions taken

| Topic | Choice | Reason |
|---|---|---|
| Network role | **Client**: IPv4 over DHCP, IPv6 over SLAAC via the upstream router RAs | no forwarding/NAT to maintain; the node is a worker like the others |
| DHCP client | **dhcpcd** (Buildroot `BR2_PACKAGE_DHCPCD`, v10.2.4) | a single binary = DHCPv4 + DHCPv6 + **RA/SLAAC** + `/etc/resolv.conf` (RDNSS included). No systemd |
| Kubernetes role | **Worker only**, joining an existing CP | the CP lives elsewhere; **no kubeadm** in the current cluster → the agent performs the join through the node config |
| Base OS | **No distro**: Buildroot (a build system, not a distro) + **glibc** | glibc is imposed by the official kubelet (dynamically linked) |
| Kernel | Compiled from **`raspberrypi/linux`**, branch `rpi-6.18.y`, tag `stable_20260724` (**6.18.39**) | explicitly requested; complete RPi hardware support (overlays, video, vcgencmd) |
| Bootloader | **U-Boot mainline** `rpi_arm64_defconfig`, chainloaded by the RPi firmware | booted natively; **A/B and rollback handled by the initramfs** (§6.4) |
| Slot selection | **File in the state partition (p4)**, read by the initramfs — not the U-Boot env | atomic write, and the rollback works **even if the agent does not start** (§6.4) |
| Init / supervision | **s6-linux** (Buildroot `BR2_PACKAGE_S6`) | native supervision, no systemd |
| Update | **A/B squashfs** + overlayfs for `/etc`, automatic rollback | atomic, immutable image, safe fallback |
| Trust | **cosign key-pair**, public key embedded → 100 % offline verification | no dependency on Fulcio/Rekor/TUF on the node; the TUF root is the embedded key |
| Update transport | **OCI registry (GHCR)** | standard cloud-native, reusable |
| Boot support | **microSD card** | requested; see §8 (wear) |
| PVC storage | **local-path, NFS, hostPath** (USB devices) | requested; kernel modules already present (`NFS_FS=y`) |
| Node bootstrap | **Dedicated DHCP option** → node config URL, **persisted at first boot** | consistent with DHCP; no secret in the image; **the node reboots without network** (§6.2). Kept as-is (decision of 2026-11-04, constrained environment) |
| Time sync | **chrony** (`BR2_PACKAGE_CHRONY`), servers from DHCP option 42 + `ntpServers` in the node config; kubelet gated on a time-synced marker | no systemd-timesyncd (no systemd); the Pi 5 has no battery-backed RTC → after a power cut the clock is wrong and the kubelet TLS to the API server fails (§5.7) |
| Boot resilience | provisioning retried every 30 s; **full reboot after 10 min** of failures, only when no persisted config exists | a full reboot re-initialises the PHY/DHCP/switch path, recovering cases a process-level retry cannot; a provisioned node never reboots for a network reason (§5.8) |
| Sysctls | **The OS sets `ip_forward` (v4+v6) + `accept_ra=2` + `rp_filter=0`**; the rest is set by kubelet/Cilium | `accept_ra` has a functional default **inverted** when forwarding is on: without `=2`, IPv6 SLAAC breaks silently (§6.5) |
| Metrics port | **12249** (not 9100) | 9100 is the conventional node_exporter port; 12249 is free |
| CI | **GitHub Actions**, native arm64 runner `ubuntu-24.04-arm` + native Arch script | GHA is possible and arm64 means no emulation |
| Signing / hashing | **cosign** (key-pair) + **SHA-256** of each artifact in CI | explicitly requested |
| Consoles | **HDMI (`tty1`) and serial (`ttyAMA0`, 115200)** | the node is usually headless but the screen is the easiest way to read the boot |
| Node name | **`kubeletFlags.nodeName`**, applied by the agent to the hostname **and** passed as `--hostname-override` | the kubelet derives the node object name from the hostname; both must agree with the name the cluster expects |

## 3. Technical constraints discovered (they drive the design)

### 3.1 Facts verified up front (do not rediscover them)

1. **The kubelet is dynamically linked against glibc.** `kubectl`/`kubeadm`/`kube-proxy` are static, but the kubelet is **excluded** from the Kubernetes `KUBE_STATIC_BINARIES` list → **glibc is mandatory**, musl is ruled out.
2. **containerd 2.4.2 does not bundle `runc`.** The tarball `containerd-static-2.4.2-linux-arm64.tar.gz` contains `containerd`, `containerd-shim-runc-v2`, `ctr`, `containerd-stress` — **`runc` must come separately** (runc v1.5.2, static `runc.arm64` binary).
3. **containerd 2.x requires cgroup v2.** Without systemd → `--cgroup-driver=cgroupfs` for the kubelet and `SystemdCgroup = false` for containerd.
4. **The RPi5 defconfig (`bcm2712_defconfig`) builds as `=m` everything the boot path needs**: `SQUASHFS=m`, `OVERLAY_FS=m`, `IPV6=m`, `NF_TABLES=m`, `NF_CONNTRACK=m`, `BRIDGE=m`, `VETH=m`, `BRIDGE_NETFILTER=m`. A kernel fragment is **mandatory** (§9 step 4). Notably `CONFIG_SECCOMP` **does not appear** in the defconfig: it is `=y` by default via `HAVE_ARCH_SECCOMP` but **runc depends on it**, so it is forced explicitly in the fragment.
5. **There is no `rpi_5_defconfig` in U-Boot v2026.10**, but `board/raspberrypi/rpi/rpi.c` **supports the Pi 5** (revision `0x17`, `bcm2712-rpi-5-b.dtb` and `bcm2712-d-rpi-5-b.dtb`) through `rpi_arm64_defconfig`, which uses the DTB provided by the firmware.
6. **The RPi5 EEPROM holds the bootloader, but the GPU firmware (`start4.elf` + `fixup4.dat`) is loaded from the boot partition.** The chainload is done through `config.txt`: `kernel=u-boot.bin` + `arm_64bit=1` (mandatory, otherwise the firmware assumes 32 bits because `u-boot.bin` is not a known kernel name). The firmware passes its DTB via `x0` (`fw_dtb_pointer` in `rpi.c`). *This corrects an earlier assumption of this plan: without `start4.elf`/`fixup4.dat` in p1, the board does not boot at all.*
7. **U-Boot v2026.10 exposes `CONFIG_BOOTMETH_RAUC`** (native A/B with an attempt counter, `BOOT_ORDER` / `BOOT_<slot>_LEFT`, automatic reset when all the slots are zero) — an alternative to the classic `bootcount`/`altbootcmd`. **Not retained**: its mechanism assumes a **boot partition per slot** (`boot_part`, `root_part` per slot) and a `boot.scr` per slot, which contradicts our layout (a single FAT p1, slots only on the rootfs). See §6.4 for the retained alternative.
8. **`cmdline.txt` does not support `kernel=`/`initramfs=`**; the kernel selection is done in `config.txt` (`kernel`, `initramfs`, `os_prefix`).
9. **dhcpcd**: the vendor option 224 is known under the name `site_specific_224` (type `binhex`). For a readable URL, `define 224 string nrzube_config_url` + `option nrzube_config_url` → the `new_nrzube_config_url` environment variable for the hook. The `define` **replaces** the built-in definition of the same option code (verified in the dhcpcd 10.2.4 sources), so the value is plain text and not hex encoded.
10. **The RPi5 kernel runs with 16 K pages** (`CONFIG_ARM64_16K_PAGES=y`, `CONFIG_LOCALVERSION="-v8-16k"`). The Go binaries (containerd, kubelet, runc) support it; still to be validated on hardware (Phase 1 gate).
11. **A 4 K page size variant exists, but as a Buildroot file**: `board/raspberrypi/linux-4k-page-size.fragment` in the **Buildroot** tree (not in the kernel tree). Switching to 4 K pages is possible if a third-party binary turns out to be incompatible — keep in mind, not the default.
12. **The kernel 6.18 is ≥ 6.17 ⇒ the runc/kernel bug is active.** runc v1.5.2 works around it. **Hard constraint**: `runc` ≥ 1.5.2 is mandatory, otherwise random crashes and memory corruption when configuring the cgroup v2 device rules.

### 3.2 Kernel choice: `rpi-6.18.y` @ `stable_20260724` (6.18.39)

The kernel is compiled from **`raspberrypi/linux`**, branch **`rpi-6.18.y`**, pinned on tag **`stable_20260724`** → version **6.18.39** (verified: the tag's `Makefile` → `VERSION 6`, `PATCHLEVEL 18`, `SUBLEVEL 39`).

Consequences:

- The `rpi-6.18.y` branch is **mobile** (HEAD was 6.18.60 when this plan was written): **the commit/tag must be pinned**, not the branch. `versions.lock` carries the exact SHA, and the build verifies it.
- `bcm2712_defconfig` exists for 6.18 and remains the starting point (16 K pages, `CONFIG_CGROUP_BPF=y`, `CONFIG_MEMCG=y`, `CONFIG_CFS_BANDWIDTH=y`, `CONFIG_CGROUP_FREEZER=y`, `CONFIG_BPF_SYSCALL=y`, `CONFIG_NET_CLS_CGROUP=y` — everything needed for cgroup v2 is already there).
- The RPi firmware must provide a **compatible DTB**: `bcm2712-rpi-5-b.dtb` / `bcm2712-d-rpi-5-b.dtb`. We use **the DTB the firmware passes via x0** (already done by `rpi.c`), not a separately compiled DTB → no DTB/kernel skew risk. The firmware `1.20260915` is more recent than the kernel 6.18.39: **check that the firmware DTB is compatible with a 6.18 kernel** (Phase 1 gate). If it is not, the fallback is to compile the DTB from the kernel tree (`BR2_LINUX_KERNEL_DTS_SUPPORT` + `BR2_LINUX_KERNEL_INTREE_DTS_NAME="bcm2712-rpi-5-b"`) and to pass it explicitly to U-Boot; the kernel-built DTB is already shipped in p1 for that purpose.

### 3.3 runc / kernel ≥ 6.17 constraint

runc v1.5.2 release notes: *"Worked around a Linux kernel bug (present since kernel v6.17, fixed in v7.2) which caused the kernel to write past the end of the structure provided by userspace (runc). This resulted in memory corruption inside runc (manifesting as random crashes) when configuring device rules on cgroup v2 systems."*

Consequence: **the 6.18 kernel is inside the affected window.** runc ≥ **1.5.2** is therefore **mandatory** (and not merely recommended). The plan pins `runc v1.5.2`: that is correct, but the wording must be "mandatory" and not "nice to have". The build refuses any older version.

## 4. Architecture

```
EEPROM (rpi-eeprom v2026.09.25-2712)
  └─ RPi firmware (raspberrypi/firmware 1.20260915)   reads p1 (FAT)
       ├─ config.txt : arm_64bit=1, kernel=u-boot.bin
       └─ loads u-boot.bin  ── firmware DTB handed over via x0 (fw_dtb_pointer)
            └─ U-Boot v2026.10 (rpi_arm64_defconfig + fragment)
                 ├─ redundant FAT env (p1): static settings only
                 └─ boot.scr: loads Image + initrd, passes root=/dev/ram0
                      └─ initramfs (BusyBox, local)  ← picks the slot
                           ├─ mounts p4 (ext4 state)
                           ├─ reads active-slot, mounts squashfs A or B (ro)
                           ├─ overlayfs /etc (upper in p4)
                           └─ switch_root → /init (s6-svscan)
                                ├─ sysctl (§6.5)
                                ├─ dhcpcd        (DHCPv4 + RA/SLAAC)
                                ├─ containerd    (SystemdCgroup=false)
                                ├─ kubelet       (--cgroup-driver=cgroupfs)
                                └─ nrzube-agent  (health + OTA + rollback)
```

### 4.1 microSD card layout (GPT)

| # | Type | Size | Content |
|---|---|---|---|
| p1 | FAT32 | 512 MiB | `config.txt`, `start4.elf`, `fixup4.dat`, `u-boot.bin`, `Image`, `initrd.img`, `bcm2712-rpi-5-b.dtb` (+ the kernel-built DTB as a fallback), `boot.scr`, `uboot.env` (redundant) |
| p2 | squashfs | ~250 MiB | rootfs **slot A** |
| p3 | squashfs | ~250 MiB | rootfs **slot B** |
| p4 | ext4 | ~4 GiB | **state**: `/var` (containerd, kubelet), `/etc` overlay upper, `nrzube/` (node config, registry token) |

A **single** FAT partition (p1) → no ambiguity for the RPi firmware, which looks
for the boot partition.

**The A/B covers the rootfs (squashfs).** Kernel + initramfs are shared in p1 and updated atomically (temporary file + rename on FAT); the RPi firmware also provides `tryboot` as a safety net at the bootloader level.

### 4.2 Persistence and overlay

- `/usr`, `/bin`, `/sbin`, `/lib`, `/opt`: squashfs **read-only** (never written).
- `/var`: ext4 (p4) — contains `containerd/`, `kubelet/`, the container images.
- `/etc`: **overlayfs** (lower = squashfs `/etc`, upper/work = p4).
- `/run`, `/tmp`: tmpfs.
- **Point of vigilance**: overlayfs over a squashfs lower layer is not a heavily tested combination. Explicit validation gate (§9, Phase 1), with a documented fallback (§5.5).

### 4.3 Role of the slot

| Element | Slot A (p2) | Slot B (p3) |
|---|---|---|
| rootfs | squashfs mounted ro by the initramfs | same |
| `/var`, `/etc` overlay | **shared** (p4) — the state follows the node from one slot to the other | |
| selection | file `/var/lib/nrzube/active-slot` in p4 (atomic write) | |

The slot selector is a **file in the state partition**, not a U-Boot env
variable: see §6.4 for the justification (atomicity + rollback possible even
if the agent does not start).

## 5. The mini Rust agent (`nrzube-agent`)

Single binary, ~3–5 MiB, started by s6. Responsibilities:

### 5.1 Bootstrap (first boot only)

1. dhcpcd provides `new_nrzube_config_url` (DHCP option 224) → the agent fetches a JSON config.
2. The config contains: `apiServer`, `caCert`, `clientCert`/`clientKey` (or a token), `kubeletFlags`, `cni`, `registry` (URL + pull token), `cosignPublicKey` (optional, otherwise the embedded one), `ntpServers` (LAN NTP servers, fed to chrony — §5.7).
3. The agent writes: `/var/lib/kubelet/pki/*`, `/var/lib/kubelet/kubeconfig`, `/var/lib/kubelet/config.yaml` (KubeletConfiguration), `/etc/cni/net.d/*`.
4. The agent **persists the config** in `/var/lib/nrzube/node-config.json` (p4) and sets the `provisioned` marker.
5. The agent **applies the node name**: `sethostname(2)` (the running hostname, what `uname -n` shows), `/etc/hostname` (persistent, in the overlay upper of p4) and `/run/nrzube/node-name`. The name must be a valid DNS-1123 label — it is the name of the Kubernetes node object.
6. The agent then starts the kubelet service (s6 depends on the agent through `contents.d`). The kubelet is started with `--hostname-override`: without it, it would derive the name of the node object from the system hostname, which would then disagree with `kubeletFlags.nodeName` (and with the name the agent watches).

> **On the next boot the DHCP is no longer consulted for the config**: the agent re-reads `node-config.json`. This is what makes the node operational offline (§6.2). The config is re-downloaded only on **explicit request** (certificate renewal, cluster change).

### 5.2 Health

- containerd: gRPC `/grpc.health.v1.Health/Check` on `unix:///run/containerd/containerd.sock`.
- kubelet: `GET https://127.0.0.1:10250/healthz` (with the node identity).
- Exposure: `GET /metrics` (Prometheus format) and `GET /healthz` on `127.0.0.1:12249`. **Port 12249 chosen** (9100 is the conventional node_exporter port: we do not take it). 12249 is free (unassigned in `/etc/services`).
- No backend to maintain: the cluster scrapes the node.

### 5.3 OTA update + rollback

1. Periodic poll of the `nrzube-<channel>` tag in the OCI registry (GHCR). **Network failure = not fatal**, retried later.
2. Fetch the **release manifest JSON** (OCI blob) and its cosign signature.
3. **Verify the signature** with the embedded public key (§5.4), **offline**, then the **SHA-256** of each announced component.
4. Download the new rootfs into `/var/lib/nrzube/staging/` (p4), **verify the signature and the hash before writing anything into a slot**.
5. Write the new squashfs into the **inactive slot** (p2 or p3) + `fsync`.
6. Replace `Image`/`initrd.img` in p1 (temporary file + `rename()`, atomic on FAT).
7. Switch: write `pending-slot`, then `active-slot` (new) atomically in p4 (write temp + `rename()`), then reboot.
8. On the next boot: the agent validates the health (containerd + kubelet + node `Ready`) for a while (e.g. 5 min). OK → remove `pending-slot`, slot confirmed. Failure → rewrite `active-slot` with the old slot + reboot (**rollback**).
9. If the node does not start at all (rootfs broken to the point where the agent does not run): that is the case the agent cannot catch → it is the role of the **initramfs failed-boot counter** (§6.4) to switch to the other slot.

### 5.4 Signature verification — decision

Format retained: **cosign v3 key-pair + "simple signing" bundle** (ECDSA P-256 signature over the payload digest).

- Rust implementation: `p256` (ECDSA P-256) + `sha2` + `serde_json` + `pkcs8`/`pem`.
- **Correctness gate**: the CI generates the fixtures with the **real `cosign` binary** (valid signature, tampered payload, corrupted bundle) and `cargo test` checks that the agent accepts the first and rejects the other two. No format is assumed without a test.

### 5.5 Trust bootstrap

- The **cosign public key (PEM)** is embedded in the image (file `/etc/nrzube/cosign.pub`), injected by Buildroot from a CI secret.
- Rotation: a new image embeds the new key. The agent refuses an update signed by an unknown key (fail-closed).
- Documented fallback if overlayfs-over-squashfs is a problem: mount the state ext4 directly on `/etc` and have the agent refresh `/etc` on every update (merge policy to be defined: keep the local overrides listed in `/etc/nrzube/local-overrides`).

### 5.6 DHCP client: dhcpcd, not systemd

The DHCP client is **dhcpcd** (Buildroot `BR2_PACKAGE_DHCPCD`, v10.2.4). Reason: a single binary covers DHCPv4 + DHCPv6 + **RA/SLAAC** + writing `/etc/resolv.conf` (RDNSS included). Alternatives ruled out: `systemd-networkd` (excluded, no systemd), BusyBox `udhcpc` (DHCPv4 only, no RA, no RDNSS → `/etc/resolv.conf` to be handled by hand), NetworkManager (out of the question).

Configuration points:

- **RA/SLAAC**: dhcpcd handles the Router Advertisements **in user space** (`ipv6nd.c` opens a raw ICMPv6 socket filtered on `ND_ROUTER_ADVERT` and configures the addresses through netlink). It is therefore **not** dependent on the kernel's `accept_ra` to do SLAAC. `accept_ra` is still set to `2` (§6.5) for the case where the kernel also does the autoconf.
- **Double SLAAC**: if the kernel has `accept_ra=1` *and* dhcpcd does SLAAC, duplicate addresses can appear. `accept_ra=2` + user-space SLAAC stays consistent (the kernel does not add an address for a prefix it already knows), but **it must be checked at the gate**: `ip -6 addr` must show one address per prefix.
- **Vendor option 224**: dhcpcd knows it as `site_specific_224` (type `binhex`). For a readable URL: `define 224 string nrzube_config_url` + `option nrzube_config_url` → the `new_nrzube_config_url` environment variable is exposed to the `dhcpcd-run-hooks` hook, which hands it over to the agent.
- **IP stability**: dhcpcd re-uses the last known lease if the DHCP server is momentarily unreachable (`lastlease`), which limits the disruptions. The indispensable complement: **DHCP reservation by MAC** on the router side + a fixed `--node-ip` for the kubelet (§8).
- **Stable IPv6**: `slaac private` (RFC 7217) gives a stable address. *Correction:* `use_tempaddr=0` does **not** exist in dhcpcd 10.2.4 — RFC 4941 temporary addresses are only created when `slaac … temporary` is requested, which we do not do.

### 5.7 Time synchronization: chrony (no systemd-timesyncd)

The Pi 5 has **no battery-backed RTC**: after a power cut the clock restarts at
the build/epoch time, and the kubelet's TLS connection to the API server fails
(certificate "not yet valid") — the node would boot but never join the cluster.
There is no systemd, so **systemd-timesyncd is not an option**.

Decision (2026-11-04): **chrony** (`BR2_PACKAGE_CHRONY`, version pinned in
`versions.lock`).

- `makestep 1.0 -1`: large offsets are stepped immediately at startup
  (unlimited steps at boot) — exactly the power-cut case.
- Two server sources, merged:
  - **DHCP option 42** (`new_ntp_servers`): a dhcpcd hook
    (`usr/lib/dhcpcd/60-nrzube-ntp`) writes the servers as a chrony `.sources`
    file under `/run/chrony/` (tmpfs);
  - **`ntpServers`** in the node config: the agent writes them as a second
    `.sources` file under `/run/chrony/` — the LAN is controlled, so known
    servers are listed there; this also covers a boot from the persisted
    config where DHCP option 42 is absent.
- Base config `/etc/chrony.conf` (overlay lower): `sourcedir /run/chrony`,
  `rtcsync` (harmless without an RTC, useful with an RTC add-on),
  `bindcmdaddress 127.0.0.1`, no `allow` (the node serves no NTP client).
- **Kubelet gating**: the kubelet s6 service waits (bounded, e.g. 120 s,
  non-fatal afterwards) for the `/run/nrzube/time-synced` marker, written by a
  small s6 service looping on `chronyc waitsync` — the kubelet must not start
  before the clock is sane.
- Files: `buildroot/configs/nrzube_pi5_defconfig` (`BR2_PACKAGE_CHRONY=y`),
  `rootfs-overlay/etc/chrony.conf`,
  `rootfs-overlay/usr/lib/dhcpcd/60-nrzube-ntp`,
  `rootfs-overlay/usr/share/nrzube/s6/chronyd/run`,
  `rootfs-overlay/usr/share/nrzube/s6/time-sync/run`, kubelet `run` updated.
- Gate check: after a power cut, `chronyc tracking` must show a sane stratum
  before the kubelet registers the node.

### 5.8 Boot resilience: retry, then full reboot after 10 minutes

Requirement (2026-11-04): the node must survive **DHCP/HTTP unavailability during
the boot process** — retry, and after **10 minutes of failures** perform a
**full reboot**.

Scope: the deadline applies **only when the node is not yet provisioned** (no
persisted `/var/lib/nrzube/node-config.json`). A provisioned node never reboots
for a network reason (§6.2): it boots on its persisted config + `lastlease`.

Behaviour (implemented in the agent's bootstrap loop,
`agent/nrzube-agent/src/main.rs`):

| Situation at boot | Behaviour |
|---|---|
| Persisted config present | load it, write the derived files — **no network, no reboot deadline** |
| No persisted config, DHCP option 224 received, HTTP fetch OK | provision and persist (current behaviour) |
| No persisted config + no option 224 / no lease / HTTP error / timeout | retry every `BOOTSTRAP_RETRY` (30 s); once `BOOT_PROVISION_DEADLINE` (10 min) since the agent start is exceeded → `sync` + `reboot(LINUX_REBOOT_CMD_RESTART)`, with an explicit log line and a `provision_deadline_exceeded` metric |

- dhcpcd keeps `-b` (the boot is not blocked on a lease) and `lastlease`
  (short DHCP outages on a provisioned node are absorbed without any reboot).
- The reboot loop (one full reboot every ~10 min while the DHCP/HTTP config
  path is down) is **intentional**: a cold reboot re-initialises the PHY, the
  DHCP client state and the switch port, which recovers cases a process-level
  retry cannot. It is visible on the serial console and in the p4 logs.
- Implementation notes: the deadline is a `Duration` parameter of the
  bootstrap function (injectable for unit tests); the agent starts seconds
  after boot, so "agent start" ≈ "boot start". Unit test: with a zero-length
  deadline and no persisted config, the bootstrap must request a reboot
  (reboot call mocked).

## 6. Offline boot and survival to a power cut

This is the structuring constraint of the system. The target environment is **constrained and controlled, not airgapped** (decision of 2026-11-04), but an already deployed node must still **reboot and rejoin the cluster with the WAN down**, and a power cut during an update must not make it unusable.

### 6.1 What is local vs. what is network

| Need at boot | Network required? | Source |
|---|---|---|
| Bootloader, kernel, initramfs, DTB | **no** | p1 (FAT), local |
| Rootfs `/usr`, `/bin`, `/sbin` | **no** | squashfs A or B, local |
| Node config: kubeconfig, kubelet config, CNI config, registry token | **no** | persisted in p4 (ext4 state) |
| IP address + default route + IPv6 SLAAC | yes, **but the local LAN only** | DHCP + RA from the router (works without internet) |
| Time synchronization (NTP) | yes, **LAN only** | DHCP option 42 + `ntpServers` from the node config → chrony (§5.7); mandatory because the Pi 5 has no battery-backed RTC |
| Container images of the pods | yes | registry (the node starts and stays `NotReady` if the registry is unreachable) |
| OS OTA update | yes | OCI registry — **a network failure must break nothing** |

Consequence: **network access is reserved for two moments** — the *first boot*
(fetching the node config if it is not already persisted) and an *update
order*. A normal boot never needs the network.

### 6.2 Persistence of the node config (the critical point)

The DHCP provides a configuration URL. Classic mistake: re-downloading this
config at **every** boot, which makes the node network dependent.

Rule: **the node config is persisted in the state partition (p4) and reused as
is on the next boot.**

1. First boot: DHCP → option 224 → URL → the agent downloads the config → **persists** it in `/var/lib/nrzube/node-config.json` (p4) → writes the `provisioned` marker.
2. Later boots: the agent reads `/var/lib/nrzube/node-config.json`. **No network access.**
3. DHCP is no longer needed after the first boot (the IP is handled by dhcpcd alone). Option 224 can disappear without consequence.
4. Corollary (decision of 2026-11-04): the 10-minute reboot deadline of §5.8 applies **only** to a node without a persisted config. A provisioned node never reboots for a network reason.

### 6.3 Atomicity of the update (survival to a power cut)

Invariant: **at any instant, there is a complete and coherent slot, and the
slot selector is written atomically.**

Write sequence of an update:

1. The agent downloads the new rootfs into `/var/lib/nrzube/staging/` (p4, ext4) and **verifies the cosign signature + the SHA-256 before writing anything into a slot**.
2. Write the squashfs into the **inactive slot** (p2 or p3), then `fsync`.
3. Update the kernel/initramfs in p1: write into a temporary file then `rename()` (atomic on the same filesystem).
4. **Slot switch**: write `/var/lib/nrzube/active-slot` in p4 through **write to temp + `rename()`** → atomic on ext4. This is the only point that decides which slot we boot, and it is atomic.

> Why not the U-Boot env: `fw_setenv` writes `uboot.env` on **FAT**, which offers no atomicity guarantee. U-Boot does provide `CONFIG_ENV_REDUNDANT` (two copies, `uboot.env` / `uboot-redund.env`) to cover a power cut during `saveenv`, but that is *additional* protection on a channel we do not want to be critical. By putting the selector in p4 (ext4, journalled, atomic `rename()`), the switch is safe **and** readable by the initramfs — see §6.4.

**Decision: the slot selector lives in the state partition (p4), not in the U-Boot env.**

### 6.4 Who chooses the slot at boot

| | U-Boot env (`boot_slot`) | File in p4 (state) |
|---|---|---|
| Write atomicity | depends on `CONFIG_ENV_REDUNDANT` (FAT) | `rename()` on ext4 = atomic |
| Readable by the agent | `fw_setenv` (u-boot-tools) | trivial read/write |
| Rollback if the new slot is broken | possible (the agent switches back) | possible |
| **Requires the agent to run** | yes | **no (the initramfs is enough)** |

**Retained choice: the initramfs (not the agent) chooses the slot**, by
reading `/var/lib/nrzube/active-slot` in p4. Reason: if the new slot is broken
to the point where the agent does not start, **we must still be able to go
back**. The initramfs is the only component whose success does not depend on
the updated OS.

Boot layout (summary):

```
p1 (FAT)   config.txt, start4.elf, fixup4.dat, u-boot.bin, Image, initrd.img,
           bcm2712-rpi-5-b.dtb, boot.scr, uboot.env
p2 (sqfs)  rootfs slot A
p3 (sqfs)  rootfs slot B
p4 (ext4)  state: /var (containerd, kubelet), /etc overlay upper, node config,
            staging/, active-slot, pending-slot, boot-ok
```

The initramfs:

1. mounts p4 (ext4);
2. reads `active-slot` (default `A`);
3. mounts the matching squashfs **read-only**;
4. mounts the `/etc` overlay (upper in p4);
5. `switch_root`.

The rollback has **two levels**:

- **Agent level** (§5.3): the agent starts, sees that the pending slot is not healthy → rewrites `active-slot` with the old slot → reboots.
- **Initramfs level**: if the pending slot is broken to the point where the agent **does not run at all**, the initramfs detects it: `active-slot` points at a slot marked `pending` and the `boot-ok` marker (written by the agent once the health is validated) is missing → the initramfs switches to the other slot and increments a counter. If that counter exceeds N, the node is considered unrecoverable and it stops on a console (avoiding a ping-pong loop).

This second level is **implemented in the real initramfs**
(`buildroot/board/nrzube/initramfs/init`): it is the guarantee that "a power
cut during an update" never leaves a bricked node.

Implementation detail (refinement of §6.4): the initramfs counts the boots
spent on the pending slot (`pending-boot-count`) and rolls back after
`PENDING_MAX_BOOTS` (3) attempts; the number of rollbacks is counted
(`boot-fail-count`) and the node stops on a console after `MAX_ROLLBACKS` (3)
rollbacks — this avoids a ping-pong between two broken slots.

## 7. Build and trust chain (CI)

### 7.1 Pinned versions (in `versions.lock`, with SHA-256)

| Component | Version |
|---|---|
| Buildroot | `2026.08.1` (`7fdeadb3b9f12f78e5fccca914d7fef47090ab9a`) |
| Kernel | `raspberrypi/linux`, branch `rpi-6.18.y`, tag `stable_20260724` (= **6.18.39**) — see §3.2 |
| RPi firmware | `raspberrypi/firmware` `1.20260915` (must provide `bcm2712-rpi-5-b.dtb`, `start4.elf`, `fixup4.dat`) |
| EEPROM | `raspberrypi/rpi-eeprom` `v2026.09.25-2712` |
| U-Boot | `v2026.10` |
| containerd | `v2.4.2` (**static** arm64 tarball) |
| runc | `v1.5.2` (`runc.arm64`, static) — **mandatory**, see §3.3 |
| Kubernetes | `v1.37.2` (`kubelet` glibc, `kubectl` static) |
| CNI plugins | `v1.9.1` (`cni-plugins-linux-arm64-v1.9.1.tgz`) |
| cosign | `v3.1.4` |
| dhcpcd | `v10.2.4` |
| chrony | `4.8` (`33ea8eb2a4daeaa506e8fcafd5d6d89027ed6f2f0609645c6f149b560d301706`, same digest as Buildroot's `package/chrony/chrony.hash`) — §5.7 |

`kube-proxy` and `crictl` are **not** in the image (decision of 2026-11-03).

### 7.2 CI steps (`.github/workflows/release.yml`, runner `ubuntu-24.04-arm`)

1. Build the kernel (Buildroot, external tree) → `Image`, `dtb`.
2. Build U-Boot (`rpi_arm64_defconfig` + fragment) → `u-boot.bin`, `mkimage` → `boot.scr`.
3. Build the rootfs via Buildroot (glibc, s6, dhcpcd, containerd, runc, CNI, nfs-utils, e2fsprogs, overlayfs, nftables/iptables) → `rootfs.squashfs` (zstd).
4. Assemble the disk image: GPT table, p1 FAT (config.txt, start4.elf, fixup4.dat, u-boot.bin, Image, initrd, dtb, boot.scr), p2/p3 squashfs, p4 ext4 (state seed).
5. **Hashing**: SHA-256 of each component + `SHA256SUMS` of the final image.
6. **Signing**: `cosign sign-blob --key <cosign.key> --bundle` on the **release manifest JSON** (list of components + sha256 + version).
7. **Publication**: `oras push` to GHCR (OCI blob `nrzube.img` + manifest blob + signature bundle), tags `nrzube-<version>` and `latest`.
8. Publish `SHA256SUMS` as a GitHub release asset.

### 7.3 Release manifest format (the signed object)

```json
{
  "schemaVersion": 1,
  "version": "2026.11.02",
  "channel": "stable",
  "components": [
    { "name": "rootfs",  "mediaType": "application/vnd.nrzube.rootfs.v1",
      "sha256": "…", "size": 268435456, "ociDigest": "sha256:…" },
    { "name": "kernel",  "mediaType": "application/vnd.nrzube.kernel.v1", "sha256": "…" },
    { "name": "initrd",  "mediaType": "application/vnd.nrzube.initrd.v1", "sha256": "…" },
    { "name": "dtb",     "mediaType": "application/vnd.nrzube.dtb.v1",    "sha256": "…" }
  ]
}
```

The agent verifies **the signature of the manifest**, then the **sha256 of
each downloaded component**. The trust chain is therefore: `embedded public
key → manifest signature → sha256 of the components`.

### 7.4 Registry access

- The **GHCR pull token is not in the image**: it is provided by the node config (DHCP option). A public registry is also supported (no token then).
- **An unreachable registry must never prevent the boot** (§6.1): the agent treats a pull failure as non-fatal and retries later.

## 8. Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| U-Boot chainload via `kernel=u-boot.bin` not validated | **blocking** | **hardware gate in §9, Phase 1 step 7**: this is the first gate, before everything else |
| **The node re-downloads its config at every boot** | **node unusable offline** | config **persisted** in p4, reused on the next boot (§6.2) — explicit offline gate |
| **IPv6 forwarding on ⇒ `accept_ra` off ⇒ SLAAC broken** | node without IPv6 | `net.ipv6.conf.all.accept_ra=2` set by the OS (§6.5); checked at the gate |
| Overlayfs over a squashfs lower layer is barely tested | broken boot | dedicated gate + documented fallback (§5.5) |
| 16 K pages vs. an incompatible binary | subtle crash | gate: boot + run of containerd/runc/kubelet on hardware |
| Unstable node IP (DHCP) | stale `Node` object, ghost pods | **DHCP reservation by MAC** on the router side + an explicit `--node-ip`; SLAAC: `slaac private` for a stable IPv6 |
| `kubelet` linked against glibc | — | glibc rootfs assumed |
| containerd 2.x without runc | containerd does not start | runc v1.5.2 installed explicitly |
| **runc < 1.5.2 on a kernel ≥ 6.17** | **random crashes + memory corruption** | **runc v1.5.2 mandatory** (§3.3) |
| cgroup v2 required | kubelet/containerd refuse to start | kernel fragment: `CONFIG_CGROUPS=y` + the controllers; kubelet `--cgroup-driver=cgroupfs`, containerd `SystemdCgroup=false` |
| microSD wear (container images, `/var`) | card death | documented; propose a persistent state on NVMe/USB (the `state` partition can be moved) — see §10 question 5 |
| Signing secret in the image | leak of the **private** key | only the **public** key is embedded; the private key only lives in the CI secrets |
| **The boot partition lacks the RPi GPU firmware** | the board does not boot | fixed in the boot gate: `start4.elf` + `fixup4.dat` are shipped in p1 (§0.4) |
| **No battery-backed RTC on the Pi 5 ⇒ after a power cut the clock is wrong ⇒ TLS to the API server fails** | node boots but never joins | chrony with `makestep` at startup + kubelet gated on a time-synced marker (§5.7) |
| **DHCP/HTTP config path unavailable at first boot** | node stuck unprovisioned | retry every 30 s, then **full reboot after 10 min** (§5.8); once provisioned, the persisted config makes later boots network-independent (§6.2) |

## 9. Execution plan

**Phase 0 — Foundations** ✅ done

1. Repository: `README.md`, `Makefile`, `.gitignore`, `docs/`.
2. `versions.lock`: all the versions + the SHA-256 of the upstream artifacts (no implicit version).

**Phase 1 — Boot gate (hardware, blocking for the rest)** 🟡 in progress

3. External Buildroot tree + `nrzube_pi5` defconfig: glibc, external kernel `raspberrypi/linux` (`BR2_LINUX_KERNEL_CUSTOM_GIT`, `BR2_LINUX_KERNEL_USE_DEFCONFIG` + `BR2_LINUX_KERNEL_DEFCONFIG="bcm2712"`, `BR2_LINUX_KERNEL_DTS_SUPPORT` + `BR2_LINUX_KERNEL_INTREE_DTS_NAME="broadcom/bcm2712-rpi-5-b"`), squashfs zstd. ✅
4. Kernel fragment (§3.1 point 4 + §3.3 + the HDMI console). ✅
5. U-Boot: `rpi_arm64_defconfig` + fragment (chainload, redundant FAT env, `boot.scr`). ✅
6. Image assembly script (GPT, p1..p4) + minimal `config.txt`. ✅ (the boot gate variant is built; the full variant is written but never built)
7. **Validation on a Pi 5**: boot → U-Boot → kernel → shell. ⏳ **pending hardware**

**Phase 2 — Rootfs** 🟡 written, never assembled

8. Full Buildroot defconfig: `BR2_PACKAGE_S6`, `BR2_PACKAGE_DHCPCD`, `BR2_PACKAGE_IPTABLES`/`NFTABLES`, `BR2_PACKAGE_IPROUTE2`, `BR2_PACKAGE_E2FSPROGS`, `BR2_PACKAGE_NFS_UTILS`, `BR2_PACKAGE_CURL`, `BR2_PACKAGE_CA_CERTIFICATES`, `BR2_PACKAGE_UBOOT_TOOLS` (fw_setenv). ✅ written
9. Buildroot overlay: `/init`, `dhcpcd.conf` (option 224), s6 services (`dhcpcd`, `containerd`, `kubelet`, `nrzube-agent`), `containerd/config.toml`, sysctl script (§6.5). ✅ written
9a. chrony: `BR2_PACKAGE_CHRONY=y` in the defconfig, `etc/chrony.conf`, dhcpcd hook `60-nrzube-ntp` (option 42), s6 services `chronyd` + `time-sync`, kubelet `run` gated on `/run/nrzube/time-synced`, agent writes `/run/chrony/nodeconfig.sources` from `ntpServers`. ✅ done 2026-11-04 (`cargo test`: 35 passed; `sh -n` on the shell files) (§5.7)
10. Buildroot package `nrzube-agent` (Rust): `Config.in` + `.mk` (cargo cross-compile aarch64). ✅ written
11. **Initramfs**: mounts p4, reads `active-slot`, mounts the squashfs + the `/etc` overlay, `switch_root`. Includes the **failed-boot counter** → automatic switch to the other slot (§6.4). ✅ written

**Phase 3 — Rust agent** 🟡 written and unit tested, never run on hardware

12. Rust workspace: config, `/metrics` + `/healthz` on **12249**, containerd/kubelet scraping. ✅
13. Bootstrap: read the DHCP option → node config → **persist in p4** → write kubeconfig/pki/kubelet config/CNI. ✅
13a. Boot provisioning deadline: `BOOT_PROVISION_DEADLINE` (10 min) + full reboot when exceeded and no persisted config; injectable deadline + unit test. ⬜ (§5.8)
14. OTA: OCI pull, cosign verification (§5.4), write the inactive slot, switch, validate, rollback. ✅

**Phase 4 — Kubernetes** ⬜ not started

15. Kubelet config (cgroupfs, node-ip, runtime endpoint), CNI plugins in `/opt/cni/bin`, kube-proxy per §10. Note: `kubeletFlags.nodeIp` is parsed by the agent but not yet emitted (`nodeIP` / `--node-ip`) — to fix here.

**Phase 5 — CI & publication** ⬜ not started

16. GHA workflow `ubuntu-24.04-arm`: full build, SHA-256, `cosign sign-blob`, `oras push` to GHCR, `SHA256SUMS` asset.
17. Local build script (Arch): native build implemented (`scripts/native-build.sh`), container build implemented (`scripts/local-build.sh`, needs a container engine with access to the docker socket).

**Phase 6 — Docs & hardening** 🟡 partial

18. `docs/architecture.md`, `docs/boot-chain.md`, `docs/ota.md`, `docs/troubleshooting.md`. ⬜ (`docs/gates.md` done)
19. Tests: agent unit tests (including the cosign fixtures §5.4) ✅, smoke-test script of the image (`boot → slot switch → rollback`) ⬜

**Phase 7 — Offline gate (to validate before declaring the system operational)** ⬜

20. On a deployed node: cut the WAN (the LAN stays up) → reboot → the node must start, get its IP, and **join the cluster** with its persisted config.
21. Cut the power **during** an update (slot being written) → reboot → the node must start on the previous, intact slot.
22. Cut the power **after** the slot switch → reboot → the agent checks the health, validates the slot; if the slot is broken, automatic rollback.

## 10. Open questions (to be settled before Phase 4)

> Environment clarified on 2026-11-04: **constrained and controlled, not airgapped**. The LAN provides DHCP, the HTTP config server, NTP and the cluster API; GHCR is reachable for OTA. WAN-down resilience is kept as a robustness requirement (§6), not as the primary operating mode.

1. **CNI / kube-proxy of the existing cluster.** *Partially answered (2026-11-03):* the cluster provides the CNI (DaemonSet) and kube-proxy (DaemonSet or replaced by Cilium). The image ships only the CNI plugin **binaries** (`/opt/cni/bin`); the configuration is written by the cluster. The image does **not** ship kube-proxy.
2. **Real join mechanism.** The DHCP delivers a config URL; but the exact way to obtain a valid kubeconfig (bootstrap token, client certificate, pre-provisioned file) must be aligned with the existing cluster. Working assumption: `clientCert` + `clientKey` in the node config.
3. **Router support for DHCP option 224.** *Decided (2026-11-04): keep the DHCP option 224 → HTTP config URL.* The router must still be confirmed to inject option 224; the documented fallback remains a `node.conf` file in p1.
4. **GHCR registry.** Public or private repository? If private, the pull token is delivered by the node config. (Environment not airgapped: GHCR is reachable.)
5. **Persistence on external storage.** The `state` partition (containerd + kubelet + local-path PVCs) on microSD will wear out. Should a layout supporting a `state` on NVMe/USB be designed from Phase 1 (design option, not decided yet).
6. **LAN NTP servers (new, 2026-11-04).** Addresses of the NTP servers of the controlled LAN, delivered via DHCP option 42 and/or `ntpServers` in the node config (§5.7).

## 11. Out of scope

- Kubernetes control plane (managed elsewhere).
- Multi-OS / multi-architecture (arm64 only).
- Encryption of the `state` partition (not requested).
- Secure boot / signed boot (the EEPROM is not updated by the image).

## 12. Build system notes (discovered while implementing)

### 12.1 Buildroot 2026.08 specifics

- The external tree needs `external.desc` (`name: NRZUBE`, `desc: …`) and `external.mk` (`include $(sort $(wildcard $(BR2_EXTERNAL_NRZUBE_PATH)/package/*/*.mk))`); the path variable is `BR2_EXTERNAL_NRZUBE_PATH` (the old `BR2_EXTERNAL_DIR_PATH` is gone).
- The kernel symbols were renamed: `BR2_LINUX_KERNEL_CUSTOM_REPO_URL` / `BR2_LINUX_KERNEL_CUSTOM_REPO_VERSION`, and `BR2_LINUX_KERNEL_DEFCONFIG` takes the name **without** the `_defconfig` suffix (`bcm2712`).
- A custom git kernel does not declare the kernel headers series → the glibc option of the internal toolchain becomes invisible and uClibc is silently selected. The **external Bootlin glibc toolchain** is used instead (`BR2_TOOLCHAIN_EXTERNAL=y` must be set explicitly: it is the member of the toolchain choice).
- `BR2_DOWNLOAD_FORCE_CHECK_HASHES=y` cannot be used (Buildroot then also wants a digest for the archive it generates from the kernel git repository).
- The U-Boot package no longer exists in Buildroot: U-Boot is built out of tree by `scripts/build-uboot.sh` from the pinned tarball (the SHA-256 is verified; a git checkout is avoided because some hosts rewrite `https://github.com/` to SSH).
- `host-mtools` is **not** built (its `floppyd` tool needs the X11 headers): the host `mcopy` is used instead.
- The host `openssl` is required by the kernel (`extract-cert`): `BR2_LINUX_KERNEL_NEEDS_HOST_OPENSSL=y`.
- The staging sysroot of an external toolchain is populated after the `lib` symlinks are created → the install fails; `buildroot/external.mk` copies the sysroot first.
- `boot.scr` is only regenerated when the host tools are rebuilt → `scripts/build-all.sh` regenerates it on every build.

### 12.2 Nix hosts

- The "format" hardening flag (`-Werror=format-security`) breaks the build of GCC's own libcpp → the flag is removed from `NIX_HARDENING_ENABLE` for the build.
- The Nix loader does not find the system `libstdc++` → `LD_LIBRARY_PATH=/usr/lib` is exported for the build.
