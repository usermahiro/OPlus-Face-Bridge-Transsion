#!/bin/bash
# Assembles the ROM-ready zip from a built binary.
# Usage: scripts/ci_package.sh <binary> <out_dir> [probe_binary]
# Env  : HAL_NAME SERVICE_NAME RC_NAME VERSION (all optional)
set -euo pipefail
BIN="${1:?usage: ci_package.sh <binary> <out_dir> [probe_binary]}"
OUT="${2:?usage: ci_package.sh <binary> <out_dir> [probe_binary]}"
PROBE="${3:-}"
HERE="$(dirname "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")")"

HAL_NAME="${HAL_NAME:-android.hardware.biometrics.face-service.default}"
SERVICE_NAME="${SERVICE_NAME:-vendor.face-default}"
RC_NAME="${RC_NAME:-android.hardware.biometrics.face-service.rc}"
VERSION="${VERSION:-dev}"
FQNAME="android.hardware.biometrics.face.IFace/default"

PKG="face-hal-rom-$VERSION"
STAGE="$OUT/$PKG"
rm -rf "$STAGE"
mkdir -p "$STAGE/vendor/bin/hw" "$STAGE/vendor/etc/init" "$STAGE/vendor/etc/vintf" "$STAGE/config"
install -m 0755 "$BIN" "$STAGE/vendor/bin/hw/$HAL_NAME"

# Optional diagnostic: never installed into /vendor, never started by init. It
# is here so the same zip can answer "can this HAL draw on its own" on a device
# where system_server cannot mint a preview token.
if [[ -n "$PROBE" ]]; then
    [[ -f "$PROBE" ]] || { echo "probe binary not found: $PROBE"; exit 1; }
    mkdir -p "$STAGE/tools"
    install -m 0755 "$PROBE" "$STAGE/tools/face-layer-probe"
    cat > "$STAGE/tools/README_PROBE.txt" <<'TXT'
face-layer-probe is a diagnostic, not part of the HAL. It creates a
SurfaceFlinger layer of its own, fills it with magenta and holds it for 30
seconds, then exits. Nothing is mounted and no service is restarted: run it
straight from /data/local/tmp.

    sh tools/run-probe.sh

Or by hand, as the same uid the HAL uses:

    cp tools/face-layer-probe /data/local/tmp/facework/
    chmod 755 /data/local/tmp/facework/face-layer-probe
    /data/local/tmp/facework/face-layer-probe

Expected lines, in this order:

    createSurfaceChecked ret=0
    connect(CPU) ret=0
    dequeueBuffer ret=0
    GraphicBuffer::lock ret=0 addr=... bpp=4 bytes_per_stride=...
    queueBuffer ret=0

Then a magenta rectangle stays on screen for 30 seconds. A missing symbol is
logged by name. See README.md, "Preview probe".
TXT
    cat > "$STAGE/tools/run-probe.sh" <<'TXT'
#!/system/bin/sh
# run-probe.sh - run the layer diagnostic without mounting anything.
# The probe creates its own SurfaceFlinger layer, so it needs no /vendor path
# and no init service: /data/local/tmp is enough.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
DST=/data/local/tmp/facework/face-layer-probe

SRC=
for c in "$HERE/face-layer-probe" "$HERE/tools/face-layer-probe" "$HERE/../tools/face-layer-probe"; do
    if [ -f "$c" ]; then
        SRC=$c
        break
    fi
done
if [ -z "$SRC" ]; then
    echo "face-layer-probe tidak ada di $HERE, $HERE/tools, atau $HERE/../tools" >&2
    exit 1
fi

mkdir -p /data/local/tmp/facework
cp -f "$SRC" "$DST"
chmod 0755 "$DST"

echo "[probe] pakai $SRC"
echo "[probe] kotak magenta tampil 30 detik, lihat layar sekarang"
"$DST" || echo "[probe] exit=$?"
echo "[probe] log:"
logcat -d -s FaceHalHW | grep -i "layer-probe" | tail -n 25
TXT
    chmod 0755 "$STAGE/tools/run-probe.sh"
fi

cat > "$STAGE/vendor/etc/init/$RC_NAME" <<RC
service $SERVICE_NAME /vendor/bin/hw/$HAL_NAME
    interface aidl $FQNAME
    # class hal balapan dengan mount /data (template wajah ada di /data/vendor_de)
    class late_start
    user system
    group system camera input
    task_profiles ProcessCapacityHigh
    capabilities SYS_NICE
    rlimit rtprio 10 10
RC

cat > "$STAGE/vendor/etc/vintf/face_aidl_fragment.xml" <<'XML'
<hal format="aidl">
    <name>android.hardware.biometrics.face</name>
    <version>3</version>
    <fqname>IFace/default</fqname>
</hal>
XML

# escaped form, as gen_file-contexts writes it
ESC_NAME="${HAL_NAME//./\\.}"
printf '/(vendor|system/vendor)/bin/hw/%s u:object_r:hal_face_default_exec:s0\n' "$ESC_NAME" \
    > "$STAGE/config/vendor_file_contexts.add"
# The label of the copy init leaves for the HAL's own re-exec. This one goes into
# the platform file_contexts: vendor_file_contexts may not label /data.
{
    printf '/data/local/tmp/facehal/%s u:object_r:hal_face_default_exec:s0\n' "$ESC_NAME"
    printf '/data/local/tmp/facehal/%s\\.unrestr u:object_r:vendor_file:s0\n' "$ESC_NAME"
} > "$STAGE/config/plat_file_contexts.add"

# Same step, as init actions: the HAL re-executes itself from /data/local/tmp
# because a vendor-class process cannot open libgui.so.
sed -e "s|@HAL_NAME@|$HAL_NAME|g" -e "s|@SERVICE_NAME@|$SERVICE_NAME|g" \
    "$HERE/rom/face-preview-prep.rc" > "$STAGE/vendor/etc/init/face-preview-prep.rc"

cat > "$STAGE/names.env" <<NAMES
HAL_NAME="$HAL_NAME"
SERVICE_NAME="$SERVICE_NAME"
RC_NAME="$RC_NAME"
FQNAME="$FQNAME"
NAMES

install -m 0755 "$HERE/rom/install_to_rom.sh" "$STAGE/install_to_rom.sh"
sed -e "s|@HAL_NAME@|$HAL_NAME|g" -e "s|@SERVICE_NAME@|$SERVICE_NAME|g" \
    "$HERE/rom/runtimetest.sh" > "$STAGE/script.sh"
chmod 0755 "$STAGE/script.sh"
sed -e "s|@HAL_NAME@|$HAL_NAME|g" -e "s|@SERVICE_NAME@|$SERVICE_NAME|g" \
    -e "s|@RC_NAME@|$RC_NAME|g" -e "s|@VERSION@|$VERSION|g" \
    "$HERE/rom/README_IMPLEMENT.md" > "$STAGE/README_IMPLEMENT.md"
cp "$HERE/LICENSE" "$HERE/THIRD_PARTY_LICENSES.md" "$STAGE/"

( cd "$STAGE" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS )
( cd "$OUT" && rm -f "$PKG.zip" && zip -qr -X "$PKG.zip" "$PKG" )
echo "packaged: $OUT/$PKG.zip"
