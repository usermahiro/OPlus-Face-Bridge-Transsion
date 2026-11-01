#!/bin/bash
# Assembles the ROM-ready zip from a built binary.
# Usage: scripts/ci_package.sh <binary> <out_dir>
# Env  : HAL_NAME SERVICE_NAME RC_NAME VERSION (all optional)
set -euo pipefail
BIN="${1:?usage: ci_package.sh <binary> <out_dir>}"
OUT="${2:?usage: ci_package.sh <binary> <out_dir>}"
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

cat > "$STAGE/names.env" <<NAMES
HAL_NAME="$HAL_NAME"
SERVICE_NAME="$SERVICE_NAME"
RC_NAME="$RC_NAME"
FQNAME="$FQNAME"
NAMES

install -m 0755 "$HERE/rom/install_to_rom.sh" "$STAGE/install_to_rom.sh"
sed -e "s|@HAL_NAME@|$HAL_NAME|g" -e "s|@SERVICE_NAME@|$SERVICE_NAME|g" \
    -e "s|@RC_NAME@|$RC_NAME|g" -e "s|@VERSION@|$VERSION|g" \
    "$HERE/rom/README_IMPLEMENT.md" > "$STAGE/README_IMPLEMENT.md"
cp "$HERE/LICENSE" "$HERE/THIRD_PARTY_LICENSES.md" "$STAGE/"

( cd "$STAGE" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS )
( cd "$OUT" && rm -f "$PKG.zip" && zip -qr -X "$PKG.zip" "$PKG" )
echo "packaged: $OUT/$PKG.zip"
