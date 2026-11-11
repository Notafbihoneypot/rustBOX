#!/bin/sh
# Fetches a Xen hypervisor binary for `cargo xtask test-xen` on Debian/Ubuntu
# without installing it: downloads the distro package, unpacks it into
# out/xen/ and gunzips the multiboot image (QEMU cannot load the .gz).
#
# Prints the path to pass as --xen / RBX_XEN.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dest="$root/out/xen"
mkdir -p "$dest"
cd "$dest"

pkg=$(apt-cache search --names-only '^xen-hypervisor-[0-9.]+-amd64$' | awk '{print $1}' | sort -V | tail -n1)
if [ -z "$pkg" ]; then
    echo "fetch-xen: no xen-hypervisor-*-amd64 package known to apt (try apt-get update)" >&2
    exit 1
fi

apt-get download "$pkg" >&2
dpkg-deb -x "$pkg"_*.deb unpacked
gz=$(ls unpacked/boot/xen-*-amd64.gz | head -n1)
gunzip -kf "$gz"
echo "$dest/${gz%.gz}"
