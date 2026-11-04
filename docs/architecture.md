# nrzube — architecture

Minimal OS image for the **Raspberry Pi 5** that boots directly on the
hardware and runs as a **Kubernetes worker node** of an existing control
plane: DHCP + IPv6 SLAAC client, **containerd + kubelet**, a Rust supervision
agent, and **signed OTA updates with automatic A/B rollback**.

There is no distro: the system is built by **Buildroot** (a build system, not
a distro) with an external **glibc** toolchain, **U-Boot** chainloaded by the
RPi firmware, and **s6-linux** as PID 1 supervisor. No systemd anywhere.

## 1. System overview

```
EEPROM (rpi-eeprom v2026.09.25-2712)
  └─ RPi GPU firmware (raspberrypi/firmware 1.20260915)   reads p1 (FAT)
       ├─ config.txt : arm_64bit=1, kernel=u-boot.bin
       └─ u-boot.bin ── firmware DTB handed over through x0
            └─ U-Boot v2026.10 (rpi_arm64_defconfig + nrzube fragment)
                 └─ boot.scr: loads Image + initrd.img, booti with the DTB
                      └─ initramfs (BusyBox, initramfs/init)  ← picks the slot
                           ├─ mounts p4 (ext4 state)
                           ├─ reads /var/lib/nrzube/active-slot
                           ├─ mounts squashfs A or B (read-only)
                           ├─ overlayfs /etc (upper in p4)
                           └─ switch_root → /init → s6-svscan
                                ├─ sysctl policy (/etc/nrzube/sysctl.sh)
                                ├─ dhcpcd        DHCPv4 + DHCPv6 + RA/SLAAC
                                ├─ chronyd + time-sync marker
                                ├─ containerd    SystemdCgroup = false
                                ├─ kubelet       --cgroup-driver=cgroupfs
                                └─ nrzube-agent  health + OTA + rollback
```

The design is driven by three constraints discovered before any code was
written (plan §3):

1. **The kubelet is dynamically linked against glibc** → glibc is mandatory,
   musl is ruled out.
2. **containerd 2.x does not bundle runc and requires cgroup v2** → runc is
   installed separately and the kernel fragment forces `CONFIG_CGROUPS=y` +
   the controllers; both runtimes use `cgroupfs`, not systemd.
3. **runc ≥ 1.5.2 is mandatory on kernel ≥ 6.17** (a kernel bug causes memory
   corruption in runc when configuring cgroup v2 device rules) → the pinned
   kernel is 6.18.39 and the pinned runc is v1.5.2.

## 2. Disk layout (GPT)

Assembled by `scripts/build-image.sh` (genimage + mksquashfs + mkfs.vfat +
mkfs.ext4):

| # | Type | Size | Content |
|---|---|---|---|
| p1 | FAT32 | 512 MiB | `config.txt`, `start4.elf`, `fixup4.dat`, `bcm2712-rpi-5-b.dtb` (firmware) + kernel-built DTB (fallback), `u-boot.bin`, `boot.scr`, `Image`, `initrd.img`, `uboot.env` (redundant) |
| p2 | squashfs (zstd) | ~250 MiB | rootfs **slot A** |
| p3 | squashfs (zstd) | ~250 MiB | rootfs **slot B** |
| p4 | ext4 | ~4 GiB | **state**: `/var` (containerd, kubelet, images), `/etc` overlay upper, `/var/lib/nrzube/` (node config, `active-slot`, `pending-slot`, `boot-ok`, staging) |

Key facts about the boot partition:

- The **GPU firmware is loaded from p1**: without `start4.elf`/`fixup4.dat`
  the EEPROM bootloader cannot start the firmware and the board stays dead.
  The chainload is done through `config.txt` (`kernel=u-boot.bin` +
  `arm_64bit=1` — mandatory, otherwise the firmware assumes 32 bits).
- A **single** FAT partition → no ambiguity for the firmware, which finds
  the boot partition by content (`config.txt`).
- The **A/B slots cover only the rootfs** (p2/p3). Kernel + initramfs are
  shared in p1 and updated atomically (temp file + `rename()` on FAT); the
  RPi firmware `tryboot` remains available as a bootloader-level safety net.

### Persistence model

- `/usr`, `/bin`, `/sbin`, `/lib`, `/opt`: squashfs, **read-only**.
- `/var`: ext4 (p4) — containerd, kubelet, container images, agent state.
- `/etc`: **overlayfs** (lower = squashfs `/etc`, upper/work in p4).
- `/run`, `/tmp`: tmpfs.

The **slot selector lives in p4** (`/var/lib/nrzube/active-slot`, written
atomically with temp + `rename()` on journalled ext4), **not** in the U-Boot
environment: FAT offers no atomicity guarantee, and the initramfs must be
able to read the selector — and roll back — **even if the agent never
starts**. This is what makes a power cut during an update unable to brick a
node.

## 3. Boot chain in detail

1. **EEPROM → firmware**: the EEPROM holds the bootloader; the firmware
   (`start4.elf`/`fixup4.dat`) is read from p1 and starts U-Boot via
   `config.txt`. The firmware passes its DTB to U-Boot through `x0`
   (`fw_dtb_pointer` in `board/raspberrypi/rpi/rpi.c`), so there is no
   DTB/kernel skew by construction; the kernel-built DTB in p1 is a fallback.
2. **U-Boot** (`rpi_arm64_defconfig` + `buildroot/board/nrzube/uboot-nrzube.fragment`):
   redundant FAT environment (static settings only), then `boot.scr`
   (generated from `board/nrzube/boot.cmd`) loads `Image` + `initrd.img` and
   boots with `booti`, `root=/dev/ram0`.
3. **Initramfs** (`buildroot/board/nrzube/initramfs/init`, BusyBox): mounts
   `/proc`, `/sys`, `/dev`; discovers and mounts p4 by its `nrzube-state`
   marker; reads `active-slot` (default `A`); runs the **failed-boot
   counter** (§6); mounts the matching squashfs read-only plus the `/etc`
   overlay (upper in p4); `switch_root` to the real system. On unrecoverable
   failure it drops to a shell on the console (it is PID 1).
4. **PID 1** (`buildroot/board/nrzube/rootfs-overlay/init`): mounts the
   volatile filesystems, loads `/etc/modules`, applies the sysctl policy
   **before any service starts** (`/etc/nrzube/sysctl.sh`: `ip_forward` v4+v6,
   `accept_ra=2` — without it IPv6 SLAAC breaks silently when forwarding is
   on — and `rp_filter=0`), copies the service definitions from the read-only
   rootfs into `/run/s6` (tmpfs) and `exec s6-svscan`.

Consoles: HDMI (`tty1`, needs `CONFIG_DRM_VC4=y`) and serial
(`ttyAMA0`, 115200, the last `console=` entry so `/dev/console` stays on the
UART).

## 4. Supervision: the s6 service tree

Service definitions live in the read-only rootfs
(`buildroot/board/nrzube/rootfs-overlay/usr/share/nrzube/s6/`) and are copied
to `/run/s6` at boot:

| Service | Role |
|---|---|
| `nrzube-base` | base setup (runtime dirs) |
| `tmpfs-logs` | `/var/log/nrzube` on tmpfs |
| `dhcpcd` | DHCPv4 + DHCPv6 + user-space RA/SLAAC + `/etc/resolv.conf` (RDNSS included); `-b` (boot not blocked on a lease) + `lastlease` |
| `chronyd` | NTP client (§5) |
| `time-sync` | writes `/run/nrzube/time-synced` once `chronyc waitsync` succeeds |
| `containerd` | `SystemdCgroup = false` (`etc/containerd/config.toml`) |
| `kubelet` | `--cgroup-driver=cgroupfs --hostname-override=<node name>`; waits (bounded, non-fatal) for the `time-synced` marker |
| `nrzube-agent` | bootstrap, health, metrics, OTA (§6); the kubelet service depends on it |

The kubelet is gated on the agent through the service dependencies: the agent
writes the kubelet configuration and identity before the kubelet starts.

## 5. Time synchronization (chrony)

The Pi 5 has **no battery-backed RTC**: after a power cut the clock restarts
at build time and the kubelet's TLS to the API server fails ("certificate not
yet valid") — the node would boot but never join. There is no systemd, so
`systemd-timesyncd` is not an option.

- **chrony** (`BR2_PACKAGE_CHRONY=y`, pinned 4.8) with `makestep 1.0 -1`
  (large offsets stepped immediately at startup — exactly the power-cut
  case), `rtcsync`, `bindcmdaddress 127.0.0.1`, no `allow`.
- Two merged server sources, both written as chrony `.sources` files under
  `/run/chrony/` (the `sourcedir` of `etc/chrony.conf`):
  - **DHCP option 42** via the dhcpcd hook
    `usr/lib/dhcpcd/60-nrzube-ntp`;
  - **`ntpServers` from the node config**, written by the agent — this also
    covers a boot from the persisted config where option 42 is absent.
- The **kubelet s6 service waits** (bounded ~120 s, non-fatal afterwards) for
  the `/run/nrzube/time-synced` marker written by the `time-sync` service.

## 6. The Rust agent (`nrzube-agent`)

Single binary started by s6, source in `agent/nrzube-agent/src/`:

| Module | Responsibility |
|---|---|
| `main.rs` | entry point, loops: bootstrap (30 s retry), health (15 s), OTA poll (15 min) |
| `config.rs` | node config: fetch, **persist** to `/var/lib/nrzube/node-config.json`, derive kubelet pki/kubeconfig/config.yaml, CNI config, containerd registry certs, chrony sources, hostname |
| `registry.rs` | minimal OCI client (no external crate): poll a tag, pull manifest + blobs, bearer-token auth |
| `cosign.rs` | cosign v3 "simple signing" bundle verification: ECDSA P-256 (`p256`) + SHA-256 over the payload digest, offline, against the embedded public key |
| `ota.rs` | update pipeline: verify signature → verify SHA-256 → stage → write inactive slot → atomic kernel/initrd replace → atomic slot switch → reboot → validate → rollback |
| `slots.rs` | atomic slot-selector read/write (temp + `rename()`), pending/boot-ok markers |
| `health.rs` | containerd gRPC health check on the unix socket + kubelet `https://127.0.0.1:10250/healthz` |
| `metrics.rs`, `server.rs` | Prometheus `/metrics` + `/healthz` on **`127.0.0.1:12249`** (9100 is node_exporter's; deliberately not used) |
| `logging.rs`, `util.rs` | structured logging, atomic file writes, reboot helper |

### 6.1 Bootstrap (first boot) and offline behaviour

1. dhcpcd's option **224** (`define 224 string nrzube_config_url` in
   `etc/dhcpcd.conf`, replacing the built-in `binhex` definition so the value
   is plain text) is handed to the agent by the hook
   `usr/lib/dhcpcd/50-nrzube`, which writes it atomically to
   `/run/nrzube/dhcp-config-url` (write + `rename()`, so the agent never
   reads a partial URL). The hook only accepts `http(s)://` URLs.
2. The agent fetches the JSON node config (`apiServer`, `caCert`,
   `clientCert`/`clientKey`, `kubeletFlags`, `cni`, `registry` + pull token,
   `cosignPublicKey`, `ntpServers`) and **persists it in p4**
   (`/var/lib/nrzube/node-config.json`) with the `provisioned` marker.
3. The agent applies the **node name** (`kubeletFlags.nodeName`, a DNS-1123
   label = the name of the Kubernetes node object) to the running hostname
   (`sethostname`), `/etc/hostname` (overlay upper) and passes it as
   `--hostname-override` to the kubelet.
4. **From the second boot on, the persisted config is reused as is and the
   DHCP is no longer consulted for the config**: a provisioned node boots and
   rejoins the cluster with the WAN down. The config is re-downloaded only on
   explicit request (certificate renewal, cluster change).

Boot resilience for an **unprovisioned** node: the bootstrap retries every
30 s; after **10 minutes** of DHCP/HTTP failures the agent performs a **full
reboot** (`sync` + `reboot(RESTART)`), because a cold reboot re-initialises
the PHY/DHCP/switch path, recovering cases a process-level retry cannot. A
provisioned node **never** reboots for a network reason.

### 6.2 OTA update and two-level rollback

Update sequence (`ota.rs` + `slots.rs`):

1. Poll the `nrzube-<channel>` tag in the OCI registry (GHCR). A network
   failure is **not fatal**, retried at the next poll.
2. Fetch the release manifest JSON + its cosign bundle; **verify the
   signature offline** with the embedded public key
   (`/etc/nrzube/cosign.pub`, injected at image build), then the SHA-256 of
   each announced component. The trust chain is:
   `embedded public key → manifest signature → SHA-256 of the components`.
   Unknown key = fail-closed.
3. Download the new rootfs into `/var/lib/nrzube/staging/` (p4) and verify
   **before writing anything into a slot**.
4. Write the new squashfs into the **inactive slot** (p2 or p3) + `fsync`.
5. Replace `Image`/`initrd.img` in p1 (temp file + `rename()`, atomic on FAT).
6. Switch: write `pending-slot`, then `active-slot` atomically in p4, reboot.
7. On the next boot the agent validates health (containerd + kubelet + node
   `Ready`) for a bounded time (~5 min). OK → remove `pending-slot`. Failure
   → rewrite `active-slot` with the old slot + reboot (**agent-level
   rollback**).
8. **Initramfs-level rollback**: if the new slot is broken to the point where
   the agent never runs, `initramfs/init` detects that `active-slot` points at
   a `pending` slot whose `boot-ok` marker is missing, switches to the other
   slot and increments `pending-boot-count`; after `PENDING_MAX_BOOTS` (3)
   boots on the pending slot it rolls back, and after `MAX_ROLLBACKS` (3)
   rollbacks it stops on a console (no ping-pong between two broken slots).

The signature format is gated by tests: CI generates fixtures with the real
`cosign` binary (`scripts/gen-cosign-fixtures.sh`) — valid signature, tampered
payload, corrupted bundle — and `cargo test` checks the agent accepts the
first and rejects the other two.

## 7. Network

- **dhcpcd** v10.2.4 is the single network client: DHCPv4 + DHCPv6 + **RA/SLAAC
  in user space** (raw ICMPv6 socket, netlink — not dependent on the kernel's
  `accept_ra`, which is still set to `2` for the kernel autoconf case) +
  `/etc/resolv.conf` from DHCP/RDNSS. Alternatives ruled out: systemd-networkd
  (no systemd), BusyBox `udhcpc` (no RA/RDNSS), NetworkManager.
- IPv6 stability: `slaac private` (RFC 7217). Note: `use_tempaddr` does not
  exist in dhcpcd 10.2.4; temporary addresses are only created with
  `slaac … temporary`, which is not used.
- IP stability: DHCP reservation by MAC on the router + explicit kubelet
  `--node-ip`.
- The node is a **client** (no forwarding/NAT of its own beyond what
  kube-proxy/CNI need); `ip_forward` is set by the OS sysctl policy, the rest
  by the cluster's kube-proxy/CNI.
- `kube-proxy` and `crictl` are **not** in the image (cluster provides
  kube-proxy; `crictl` has no user without SSH). `kubectl` is kept for console
  debugging.

## 8. Build system and version pinning

- **Buildroot 2026.08.1 as an external tree** (`buildroot/`: `external.desc`,
  `external.mk`, `Config.in`, `configs/`, `board/nrzube/`, `package/`). Two
  defconfigs: `nrzube_pi5_defconfig` (full system) and
  `nrzube_pi5_boot_defconfig` (boot gate).
- **External Bootlin glibc aarch64 toolchain**: with a custom git kernel,
  Buildroot cannot see the kernel headers series and silently selects uClibc;
  the external glibc toolchain works around this.
- **Kernel**: `raspberrypi/linux` branch `rpi-6.18.y` frozen on tag
  `stable_20260724` (**6.18.39**), `bcm2712_defconfig` +
  `board/nrzube/kernel-nrzube.fragment` which forces the boot-path symbols to
  `=y` (`SQUASHFS`, `OVERLAY_FS`, `IPV6`, `NF_TABLES`, `NF_CONNTRACK`,
  `BRIDGE`, `VETH`, `BRIDGE_NETFILTER`, `CGROUPS`, `SECCOMP`, `DRM`,
  `DRM_VC4`, `FRAMEBUFFER_CONSOLE`). The kernel runs with **16 K pages**
  (`-v8-16k`).
- **U-Boot v2026.10** built out of tree by `scripts/build-uboot.sh`
  (`rpi_arm64_defconfig` + fragment); `boot.scr` generated from `boot.cmd`.
- **Everything is pinned in `versions.lock`**: versions, git commits (never
  branches) and SHA-256 of every downloaded artifact, with Renovate
  annotations; URLs are derived from the versions in `versions.mk`, so a bump
  can never leave a stale URL behind. `make versions-lock` re-resolves and
  re-hashes.
- **Build drivers**: `scripts/build-all.sh` (orchestration),
  `scripts/native-build.sh` (native, incl. Nix-host workarounds),
  `scripts/local-build.sh` / `build-container.sh` (containerized).
- **Boot gate artifact** (built and statically verified, not yet validated on
  hardware): `dist/nrzube-boot-gate.img` (512 MiB, GPT, one FAT32 partition
  with `config.txt`, `start4.elf`, `fixup4.dat`, both DTBs, `u-boot.bin`,
  `boot.scr`, `Image`, `initrd.img` with the stage-1 initramfs
  `initramfs/stage1-init`) + `dist/nrzube-boot-gate.img.sha256`.
- **CI** (`.github/workflows/`, native arm64 runner): build, SHA-256,
  `cosign sign-blob` on the release manifest, `oras push` to GHCR — see
  [ci.md](ci.md).

## 9. Security model

- **Trust root = the cosign public key embedded in the image**
  (`/etc/nrzube/cosign.pub`). Verification is 100 % offline (ECDSA P-256 +
  SHA-256, no Fulcio/Rekor/TUF on the node). Only the **public** key is in the
  image; the private key lives only in the CI secrets. Key rotation = a new
  image.
- **No credential in the image**: the GHCR pull token and the cluster
  credentials arrive via the node config (DHCP option 224 → HTTP) and are
  persisted only in the state partition p4 (mode 0600, atomic writes).
- No secure boot / signed boot: the EEPROM is not updated by the image (out of
  scope); the signature chain starts at the OTA manifest.
- No secret is ever written to a log or an error path.

## 10. Current status and known limits

- **Built and verified statically**: the boot gate image (§8). Everything else
  — full rootfs, real initramfs, agent — is **written and unit tested but
  never assembled or run on hardware**.
- The first open gate is the **hardware validation of the boot chain**
  (gate 1, `docs/gates.md`): GPT vs MBR discovery by the firmware, EEPROM
  version, DTB/kernel compatibility, U-Boot chainload via `kernel=u-boot.bin`.
- Overlayfs over a squashfs lower layer is a barely tested combination:
  explicit validation gate, documented fallback (mount p4 directly on `/etc`).
- microSD wear on p4 (containerd images, `/var`): documented risk; a layout
  with the state partition on NVMe/USB is a design option, not decided.
- Remaining docs: `boot-chain.md`, `ota.md`, `troubleshooting.md`.

## 11. Repository map

```
buildroot/            Buildroot external tree (br2-external)
  configs/            nrzube_pi5_defconfig (full), nrzube_pi5_boot_defconfig (gate)
  board/nrzube/       config.txt, boot.cmd, kernel/U-Boot/busybox fragments,
                      rootfs-overlay/ (init, s6 services, dhcpcd hooks, sysctl),
                      initramfs/ (init = slot selection + rollback, stage1-init)
  package/nrzube-agent/  Config.in + .mk (cargo cross-compilation)
agent/                Rust workspace: nrzube-agent (config, cosign, registry,
                      ota, slots, health, metrics, server)
scripts/              build drivers, image assembly, initramfs, fixtures, CI helpers
dist/                 built images + SHA-256
docs/                 this document, gates.md, ci.md
versions.lock         versions + commits + SHA-256 of every upstream artifact
```
