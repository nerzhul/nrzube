# nrzube

Minimal OS image for the **Raspberry Pi 5**: boots directly on the hardware,
runs a DHCP + IPv6 SLAAC client, executes **containerd + kubelet** as a worker
of an existing Kubernetes cluster, exposes a Rust supervision agent, and
applies **signed OTA updates with automatic A/B rollback**.

No distro: **Buildroot** (a build system, not a distro) + compiled **glibc**,
**s6-linux** as supervisor, **U-Boot** chainloaded by the RPi firmware.

> The reference plan (source of truth) lives in
> `.kilo/plans/1793628833208-nrzube-rpi5-k8s-node-image.md`.

## Boot chain

```
EEPROM (rpi-eeprom v2026.09.25-2712)
 └─ firmware RPi (1.20260915)          reads p1 (FAT)
     ├─ config.txt : arm_64bit=1, kernel=u-boot.bin
     └─ u-boot.bin                     firmware DTB handed over through x0
         └─ boot.scr                   loads Image + initrd.img
             └─ initramfs (BusyBox)    ← picks the slot (A/B)
                 ├─ mounts p4 (ext4 state)
                 ├─ reads active-slot, mounts the squashfs (ro)
                 ├─ overlayfs /etc (upper in p4)
                 └─ switch_root → /init (s6-svscan)
                     ├─ sysctl (plan §6.5)
                     ├─ dhcpcd        DHCPv4 + RA/SLAAC
                     ├─ containerd    SystemdCgroup=false
                     ├─ kubelet       --cgroup-driver=cgroupfs
                     └─ nrzube-agent  health + OTA + rollback
```

## SD card layout (GPT)

| # | Type | Content |
|---|---|---|
| p1 | FAT32 | `config.txt`, `u-boot.bin`, `Image`, `initrd.img`, `boot.scr`, `uboot.env` (redundant) |
| p2 | squashfs | rootfs **slot A** |
| p3 | squashfs | rootfs **slot B** |
| p4 | ext4 | **state**: `/var` (containerd, kubelet), `/etc` overlay upper, node config, OTA staging |

The **slot selector lives in p4** (`/var/lib/nrzube/active-slot`, written
atomically with `rename()`), not in the U-Boot environment: the initramfs can
therefore fall back to the other slot **even if the agent never starts** —
this is what guarantees that a power cut during an update cannot brick a node.

## Repository layout

```
buildroot/            Buildroot external tree (br2-external)
  configs/            nrzube_pi5_defconfig
  board/nrzube/       kernel/U-Boot fragments, config.txt, boot.cmd,
                      rootfs overlay (init, s6 services, sysctl, dhcpcd.conf),
                      initramfs (slot selection + failed-boot counter)
  package/            nrzube-agent (Config.in + .mk, cargo cross-compilation)
agent/                Rust workspace: config, health, metrics, OTA, cosign
scripts/              build (container), initramfs, versions.lock, fixtures
docs/                 architecture, boot-chain, ota, troubleshooting
versions.lock         versions + SHA-256 of every upstream artifact
renovate.json         automated version maintenance
```

## Quick start

```bash
# 1. cosign key pair to embed (the private key stays in the CI secrets)
mkdir -p keys && cosign generate-key-pair --output-key-prefix keys/cosign

# 2. full build (rootfs + initramfs + GPT image) inside a container
make image

# 3. artifacts
ls dist/            # nrzube.img, nrzube.img.sha256, release.json, release.json.bundle

# 4. flash onto an SD card
make flash FLASH_DEVICE=/dev/sdb
```

A native build (no container) is possible when the host has the dependencies:

```bash
make rootfs && make initramfs && ./scripts/build-image.sh
```

## Tests

```bash
make test          # cargo test (agent) + shell script syntax
make fixtures      # regenerate the cosign fixtures (valid signature, tampered
                    # payload, corrupted bundle)
```

Signature verification is **not implemented blindly**: the test fixtures are
produced by the **real cosign binary** (v3.1.4), and the bundle format
(`application/vnd.dev.sigstore.bundle.v0.3+json`) is asserted by the tests
rather than assumed.

## Documentation

| Document | Content |
|---|---|
| [docs/architecture.md](docs/architecture.md) | components, layout, persistence, A/B slots |
| [docs/boot-chain.md](docs/boot-chain.md) | firmware → U-Boot → kernel → initramfs → s6, and boot troubleshooting |
| [docs/ota.md](docs/ota.md) | manifest format, cosign signature, slot switch, rollback |
| [docs/troubleshooting.md](docs/troubleshooting.md) | symptom → diagnosis → action |
| [docs/gates.md](docs/gates.md) | validation gates (Phase 1, offline, power-cut) |

## Non-negotiable constraints

- **runc ≥ 1.5.2**: kernels ≥ 6.17 (including 6.18) trigger an out-of-bounds
  write on cgroup v2 structures → memory corruption and random crashes.
  runc v1.5.2 is pinned and the build refuses any older version.
- **`net.ipv6.conf.all.accept_ra=2`**: with IPv6 forwarding enabled the
  kernel's functional default *disables* `accept_ra` → **SLAAC silently
  breaks**.
- **The node config is persisted** in p4: the node reboots and rejoins the
  cluster **without network** (except at first boot and when an update is
  explicitly ordered).
- **`SystemdCgroup=false`** (containerd) and **`--cgroup-driver=cgroupfs`**
  (kubelet): there is no systemd on the node.
