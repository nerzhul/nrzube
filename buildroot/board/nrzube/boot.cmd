# =============================================================================
# nrzube — U-Boot boot script (compiled into boot.scr by mkimage)
#
# Executed by the bootstd "script" boot method: it looks for "boot.scr.uimg"
# then "boot.scr", with the "/" and "/boot/" prefixes, on the boot partition.
# boot.scr is placed at the root of p1.
#
# The DTB used here is the one the RPi firmware hands over through x0 (env
# fdt_addr, set by board/raspberrypi/rpi/rpi.c: set_fdt_addr()): there is
# therefore no risk of DTB/kernel skew.
# =============================================================================

# Kernel command line.
#  - console: tty1 = the video console (HDMI, vc4 framebuffer) and
#    ttyAMA0 = the RP1 UART (GPIO14/15). The kernel writes to both; the last
#    console listed (/dev/console) is the serial port, so that the
#    interactive shell stays reachable even without a screen.
#  - root=/dev/ram0: the initramfs takes over and mounts the slot squashfs
#    itself, so the kernel root is the initramfs ramfs.
setenv bootargs "console=tty1 console=ttyAMA0,115200 root=/dev/ram0 rw"

# Boot partition discovery: on a Pi 5 the SD card is on mmc 0; mmc 1 is
# tried as a fallback (CM5/eMMC). Without a kernel we stop on the U-Boot
# console instead of booting a partial system.
setenv bootdev ""

fatload mmc 0:1 ${kernel_addr_r} Image && setenv bootdev "0:1"
if test -z "${bootdev}"; then
	fatload mmc 1:1 ${kernel_addr_r} Image && setenv bootdev "1:1"
fi

if test -z "${bootdev}"; then
	echo "nrzube: Image not found on mmc 0:1 nor mmc 1:1"
	echo "nrzube: check the content of the FAT partition p1"
	reset
fi

echo "nrzube: booting from mmc ${bootdev}"

# Kernel and initramfs (shared between slots A and B: they are replaced
# atomically by the agent during an update).
fatload mmc ${bootdev} ${kernel_addr_r} Image
fatload mmc ${bootdev} ${ramdisk_addr_r} initrd.img

# The initramfs size is saved right away: ${filesize} is overwritten by the
# fallback DTB download below.
setenv initrd_size ${filesize}

# Firmware DTB; fallback to the one compiled from the kernel tree.
if test -z "${fdt_addr}"; then
	echo "nrzube: empty fdt_addr, loading the fallback DTB"
	fatload mmc ${bootdev} ${fdt_addr_r} bcm2712-rpi-5-b-kernel.dtb
	setenv fdt_addr ${fdt_addr_r}
fi

# booti expects the initramfs address, its size, then the DTB address.
booti ${kernel_addr_r} ${ramdisk_addr_r}:${initrd_size} ${fdt_addr}
