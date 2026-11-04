# Validation gates

The plan defines several gates. Each one isolates a class of failure so that
the debugging stays tractable.

| Gate | What it validates | Artifact |
|---|---|---|
| 1 | firmware → U-Boot → kernel → initramfs → shell | `dist/nrzube-boot-gate.img` |
| 2 | full system: rootfs slots, state partition, containerd, kubelet, agent | `dist/nrzube.img` |
| 3 | offline: the node boots and rejoins the cluster without the network | — |
| 4 | power cut: an interrupted update never bricks the node | — |
| 5 | OTA: signature verification, slot switch, automatic rollback | — |

---

## Gate 1 — boot chain

The goal of this gate is to validate the *hardware* part of the boot chain
before anything else is built. The image contains only:

```
p1 (FAT32)  config.txt, u-boot.bin, Image, initrd.img, boot.scr, fallback DTB
```

There is no rootfs, no state partition and no service: the initramfs mounts
the pseudo filesystems, prints the kernel banner and drops to a shell.

### Build

```bash
./scripts/native-build.sh boot-gate      # or: ./scripts/local-build.sh boot-gate
```

Artifacts:

```
dist/nrzube-boot-gate.img
dist/nrzube-boot-gate.img.sha256
```

### Flash

```bash
make flash FLASH_DEVICE=/dev/sdX        # or: dd if=dist/nrzube-boot-gate.img of=/dev/sdX bs=4M conv=fsync
```

### Checklist

| # | Check | Expected |
|---|---|---|
| 1 | The Pi boots (LED activity) | the firmware finds `config.txt` on p1 |
| 2 | The serial console (115200, GPIO14/15) shows U-Boot | `U-Boot 2026.10 (...)` |
| 3 | U-Boot loads the kernel | `nrzube: boot from mmc 0:1` |
| 4 | The kernel starts | `Linux version 6.18.39 ...` |
| 5 | The initramfs runs | `nrzube[stage1]: kernel: Linux ...` |
| 6 | The DTB comes from the firmware | `nrzube[stage1]: model: Raspberry Pi 5 Model B Rev 1.1` |
| 7 | A shell is available | the `nrzube[stage1]` banner is followed by a prompt |

If the console stays silent, the failure is in the firmware or in U-Boot
(see [boot-chain.md](boot-chain.md)).

### What to collect when it fails

```bash
# the U-Boot environment and the boot log
# (the console output is the primary source)
```

- The U-Boot banner (version, board, DRAM size).
- The `nrzube: ...` lines of the boot script.
- The kernel command line (`/proc/cmdline`).

---

## Gate 2 — full system

```bash
./scripts/native-build.sh full
```

| # | Check | Expected |
|---|---|---|
| 1 | The partition table | `p1` FAT, `p2`/`p3` squashfs, `p4` ext4 (`nrzube-state`) |
| 2 | The slot selection | `nrzube[initramfs]: booting slot A` |
| 3 | The overlay | `/etc` is an overlayfs, the upper layer is in p4 |
| 4 | The network | an IPv4 lease and an IPv6 SLAAC address |
| 5 | The SLAAC | `ip -6 addr` shows **one stable address per prefix** |
| 6 | containerd | `crictl`-less check: the socket answers, `/run/containerd` exists |
| 7 | The kubelet | the node object appears in the cluster and becomes `Ready` |
| 8 | The agent | `curl 127.0.0.1:12249/healthz` and `/metrics` answer |
| 9 | The config persistence | `/var/lib/nrzube/node-config.json` exists after the first boot |
| 10 | The time sync (chrony) | `chronyc tracking` shows a stratum < 16, `/run/nrzube/time-synced` exists, `/run/chrony/` holds `dhcp.sources` (option 42) and `nodeconfig.sources` (`ntpServers`), and the kubelet started after the marker |

---

## Gate 3 — offline

The node must boot and rejoin the cluster **without the network**:

1. Provision the node once (the configuration is delivered by the DHCP
   option 224 and persisted in p4).
2. Disconnect the network (or disable the DHCP server).
3. Reboot: the node must come up, start containerd and the kubelet, and the
   node object must become `Ready` again.

The DHCP option 224 is read **once**: after the first boot the configuration
comes from `/var/lib/nrzube/node-config.json`.

The NTP sources survive the same way: the agent rewrites
`/run/chrony/nodeconfig.sources` from `ntpServers` at every boot, so chrony
gets its sources even with the DHCP server disabled (the `dhcp.sources` file
from option 42 is then simply absent).

---

## Gate 4 — power cut during an update

This gate validates the two levels of rollback:

1. Start an update (the agent writes the inactive slot, then switches).
2. Cut the power **during the write** of the new rootfs.
3. Power the node back on: it must boot the *previous* slot (the failed-boot
   counter of the initramfs) and the node must be `Ready`.
4. Cut the power **after the slot switch** but before the agent confirms the
   new slot: the node must roll back to the previous slot after
   `PENDING_MAX_BOOTS` boots.
5. Cut the power **during the write of `Image`/`initrd.img`** in p1: the FAT
   rename is atomic, so the node must boot with either the old or the new
   kernel — never with a truncated file.

The counters are in p4:

```
/var/lib/nrzube/pending-slot       the slot waiting for its confirmation
/var/lib/nrzube/pending-boot-count the boots spent on the pending slot
/var/lib/nrzube/boot-fail-count    the rollbacks already performed
/var/lib/nrzube/boot-ok            the slot confirmed healthy
```

---

## Gate 5 — OTA

| # | Check | Expected |
|---|---|---|
| 1 | A tampered manifest is refused | the agent logs a signature error and changes nothing |
| 2 | A manifest signed by another key is refused | same |
| 3 | A valid manifest is applied | the new slot is written, the node reboots into it |
| 4 | The new slot is confirmed | `pending-slot` is removed, `boot-ok` is written |
| 5 | A broken rootfs rolls back | the node goes back to the previous slot |
| 6 | An unreachable registry is not fatal | the node keeps running, the error is logged |
