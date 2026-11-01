#!/bin/bash
# Verifies the cross-compiled HAL binary before it is packaged.
# Usage: scripts/ci_verify.sh <binary>
set -euo pipefail
BIN="${1:?usage: ci_verify.sh <binary>}"

echo "== file"; file "$BIN"
readelf -h "$BIN" | grep -q 'Machine:.*AArch64' || { echo "FAIL: not AArch64"; exit 1; }

echo "== interpreter"
INTERP=$(readelf -l "$BIN" | sed -n 's/.*Requesting program interpreter: \(.*\)\]/\1/p')
echo "$INTERP"
[[ "$INTERP" == "/system/bin/linker64" ]] || { echo "FAIL: interpreter is not /system/bin/linker64"; exit 1; }

echo "== DT_NEEDED"
readelf -d "$BIN" | awk '/NEEDED/ {gsub(/[][]/,"",$5); print $5}' | tee /tmp/needed.txt
# Every one of these resolves from a vendor process on the device. Anything
# else (libc++_shared, libstdc++, ...) would not be found in the vendor namespace.
ALLOW='^(libc\.so|libm\.so|libdl\.so|liblog\.so|libnativewindow\.so|libmediandk\.so|libcamera2ndk_vendor\.so|libstfaceunlockppl\.so)$'
bad=0
while read -r lib; do
    [[ "$lib" =~ $ALLOW ]] || { echo "FAIL: unexpected DT_NEEDED $lib"; bad=1; }
done < /tmp/needed.txt
[[ $bad -eq 0 ]] || exit 1

echo "== st_* imports"
readelf --dyn-syms "$BIN" | awk '$7=="UND" && $8 ~ /^st_/ {print "  "$8}' | sort -u | tee /tmp/st.txt
grep -q st_auth_enroll /tmp/st.txt || { echo "FAIL: st_auth_enroll not imported"; exit 1; }

echo "== licence fix"
grep -aq "continuing with an empty licence" "$BIN" || { echo "FAIL: binary tanpa kode lisensi terbaru"; exit 1; }
grep -aq "/odm/etc/asset/face/license.lic" "$BIN" || { echo "FAIL: path lisensi /odm tidak ada di binary"; exit 1; }

echo "verify OK ($(stat -c%s "$BIN") bytes)"
