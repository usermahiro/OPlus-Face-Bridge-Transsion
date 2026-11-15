#!/system/bin/sh
# script.sh - runtime test for the AIDL face HAL without repacking the ROM.
#
# init refuses to exec a binary that lives on /data, so this bind-mounts the
# built HAL over the vendor path, restarts the service and prints the log.
#
#   su -c 'sh script.sh'           mount + restart
#   su -c 'sh script.sh unmount'   drop the mount, back to the stock binary
#
# The binary is taken from vendor/bin/hw/ next to this script, so extract the
# zip first and run the script from that folder.
set -eu

HAL_NAME="@HAL_NAME@"
SERVICE_NAME="@SERVICE_NAME@"
DST="/vendor/bin/hw/$HAL_NAME"
# Files on /data sit on a nosuid mount, and SELinux then refuses the transition
# init -> hal_face_default (avc: denied nosuid_transition), so init falls back to
# execute_no_trans and dies without logging anything. A tmpfs mounted without
# nosuid does not have that problem.
MNT="${MNT:-/data/local/tmp/facemnt}"
HERE=$(cd "$(dirname "$0")" && pwd)
SRC="$HERE/vendor/bin/hw/$HAL_NAME"
MODE="${1:-mount}"

say() { echo "[script] $*"; }

[ "$(id -u)" = 0 ] || { say "butuh root: su -c 'sh $0'"; exit 1; }

# A leftover bind mount stacks: umount always wins over mount, and two entries
# on the same target leave the older file in place. Clear it first.
if grep -q " $DST " /proc/mounts; then
    say "ada mount lama di $DST, unmount dulu"
    umount "$DST" || { say "umount gagal, reboot lalu ulangi"; exit 1; }
fi

if [ "$MODE" = "unmount" ]; then
    say "kembali ke binary stock"
    stop "$SERVICE_NAME" 2>/dev/null || true
    start "$SERVICE_NAME" 2>/dev/null || true
    sleep 1
    say "status: $(getprop "init.svc.$SERVICE_NAME")"
    exit 0
fi

[ -f "$SRC" ] || { say "binary tidak ada: $SRC"; say "extract zip dulu, jalankan dari folder hasil extract"; exit 1; }

# The copy that init actually runs lives on tmpfs, not on /data.
mkdir -p "$MNT"
if ! grep -q " $MNT tmpfs " /proc/mounts; then
    mount -t tmpfs -o rw,seclabel,mode=0755 tmpfs "$MNT" \
        || { say "mount tmpfs gagal: $MNT"; exit 1; }
fi
cp -f "$SRC" "$MNT/$HAL_NAME"
chmod 0755 "$MNT/$HAL_NAME"

# init execs this inode through /vendor/bin/hw, so the label has to be the
# vendor exec label. On /data it is shell_data_file and the exec would be denied.
chcon u:object_r:hal_face_default_exec:s0 "$MNT/$HAL_NAME" 2>/dev/null \
    || say "peringatan: chcon gagal, exec bisa ditolak SELinux"

mount --bind "$MNT/$HAL_NAME" "$DST" || { say "mount --bind gagal"; exit 1; }
grep -q " $DST " /proc/mounts || { say "mount tidak terlihat di /proc/mounts"; exit 1; }
say "mount ok: $DST -> $MNT/$HAL_NAME"

# The whole point of the tmpfs: no nosuid on the filesystem holding the inode.
MOPTS=$(awk -v t="$DST" '$2==t {print $4}' /proc/mounts)
case ",$MOPTS," in
    *,nosuid,*) say "peringatan: mount masih nosuid ($MOPTS), init akan menolak transisi";;
esac

# The rc that defines the service comes from this same zip; without it there is
# nothing for init to start.
if [ -z "$(getprop "init.svc.$SERVICE_NAME")" ]; then
    say "service $SERVICE_NAME belum terdaftar"
    say "copy vendor/etc/init/*.rc dan vendor/etc/vintf/*.xml dulu (install_to_rom.sh), lalu reboot"
    exit 1
fi

# Default log buffer wraps a HAL startup away in seconds.
logcat -G 32M >/dev/null 2>&1 || true
stop "$SERVICE_NAME" 2>/dev/null || true
start "$SERVICE_NAME" 2>/dev/null || true
sleep 2

say "status: $(getprop "init.svc.$SERVICE_NAME")"
logcat -d -s FaceHalHW | tail -n 30
