#!/bin/sh
# =============================================================================
# nrzube — kernel parameter policy (plan §6.5).
#
# There is no systemd on the node, hence no systemd-sysctl: the values are
# written directly to /proc/sys. Only the parameters the cluster does NOT
# set itself are listed here — the ones set by kubelet/Cilium would create
# two sources of truth.
# =============================================================================
set -u

LOG_PREFIX="nrzube[sysctl]"

set_sysctl() {
	key="$1"
	value="$2"
	path="/proc/sys/$(printf '%s' "$key" | tr '.' '/')"

	if [ ! -e "$path" ]; then
		echo "$LOG_PREFIX: $key not available (no $path)" >&2
		return 0
	fi
	if printf '%s\n' "$value" > "$path" 2>/dev/null; then
		echo "$LOG_PREFIX: $key = $value"
	else
		echo "$LOG_PREFIX: FAILED to set $key = $value" >&2
	fi
}

# --- Forwarding --------------------------------------------------------------
# Required by kube-proxy and by every CNI that routes.
# NB: enabling IPv6 forwarding flips the kernel's *functional default* of
# accept_ra to "disabled" — see accept_ra below.
set_sysctl net.ipv4.ip_forward 1
set_sysctl net.ipv6.conf.all.forwarding 1
set_sysctl net.ipv6.conf.default.forwarding 1

# --- IPv6 router advertisements ---------------------------------------------
# 2 = "overrule forwarding behaviour": RAs are accepted even though local
# forwarding is enabled. Without this the node silently loses SLAAC.
set_sysctl net.ipv6.conf.all.accept_ra 2
set_sysctl net.ipv6.conf.default.accept_ra 2

# --- Reverse path filtering --------------------------------------------------
# Several CNIs (Cilium among them) require rp_filter to be disabled. Cilium
# sets it itself in its init container, but the OS sets it too for the CNIs
# that do not.
set_sysctl net.ipv4.conf.all.rp_filter 0
set_sysctl net.ipv4.conf.default.rp_filter 0

# --- IPv6 consistency --------------------------------------------------------
set_sysctl net.ipv6.conf.all.disable_ipv6 0
set_sysctl net.ipv6.conf.default.disable_ipv6 0

# --- Privileged ports --------------------------------------------------------
# Required by the kubelet to bind hostPorts below 1024.
set_sysctl net.ipv4.ip_unprivileged_port_start 0
