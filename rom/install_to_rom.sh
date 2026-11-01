#!/bin/bash
# install_to_rom.sh - drops the face HAL into an UNPACKED ROM tree.
# Writes files only; never repacks and never touches a device.
#
# Usage:  ROM_TREE=/path/to/erofs ./install_to_rom.sh
#         ./install_to_rom.sh /path/to/erofs
#
# ROM_TREE must contain vendor/ (and optionally config/vendor_file_contexts).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROM_TREE="${1:-${ROM_TREE:-}}"
[[ -n "$ROM_TREE" && -d "$ROM_TREE/vendor" ]] || {
    echo "ROM_TREE kosong / tidak ada folder vendor/ di: '${ROM_TREE}'"
    echo "pakai: ROM_TREE=/path/ke/erofs $0"
    exit 1
}
VENDOR="$ROM_TREE/vendor"

# shellcheck disable=SC1091
source "$HERE/names.env"   # HAL_NAME, SERVICE_NAME, RC_NAME, FQNAME

echo "ROM tree : $ROM_TREE"
echo "binary   : /vendor/bin/hw/$HAL_NAME"
echo "service  : $SERVICE_NAME"

# 1. binary ------------------------------------------------------------------
install -D -m 0755 "$HERE/vendor/bin/hw/$HAL_NAME" "$VENDOR/bin/hw/$HAL_NAME"
echo "  [ok] bin/hw/$HAL_NAME"

# 2. init rc -----------------------------------------------------------------
# Warn about any other rc that already starts a face service, otherwise two
# services fight over IFace/default.
mkdir -p "$VENDOR/etc/init"
while IFS= read -r f; do
    [[ "$(basename "$f")" == "$RC_NAME" ]] && continue
    echo "  [WARN] rc lain masih mendefinisikan service face: ${f#"$ROM_TREE"/}"
    echo "         hapus/ganti supaya tidak dobel dengan $SERVICE_NAME"
done < <(grep -rlE 'biometrics\.face|vendor\.face' "$VENDOR/etc/init" 2>/dev/null || true)
install -D -m 0644 "$HERE/vendor/etc/init/$RC_NAME" "$VENDOR/etc/init/$RC_NAME"
echo "  [ok] etc/init/$RC_NAME"

# 3. VINTF: HAL harus DIDEKLARASIKAN di manifest efektif ----------------------
# FaceService memakai waitForDeclaredService, jadi HAL yang hanya terdaftar
# tapi tidak ada di manifest tidak akan pernah ditemukan.
MANIFEST="$VENDOR/etc/vintf/manifest.xml"
if [[ -f "$MANIFEST" ]]; then
    if python3 - "$MANIFEST" "$HERE/vendor/etc/vintf/face_aidl_fragment.xml" <<'PY'
import re, sys
p, frag_path = sys.argv[1], sys.argv[2]
s = open(p, encoding="utf-8").read()
frag = open(frag_path, encoding="utf-8").read().rstrip("\n")
if re.search(r'<hal format="aidl">\s*<name>android\.hardware\.biometrics\.face</name>', s):
    print("  [ok] vintf: entri AIDL face sudah ada di manifest.xml (tidak diubah)")
    sys.exit(0)
m = re.search(r'\n([ \t]*)</manifest>\s*$', s)
if not m:
    sys.exit("  [FAIL] tidak menemukan </manifest> penutup")
ind = m.group(1) + "    "
frag = "\n".join(ind + l if l else l for l in frag.split("\n"))
s = s[:m.start()] + "\n" + frag + s[m.start():]
open(p, "w", encoding="utf-8").write(s)
print("  [ok] vintf: entri AIDL face v3 ditambahkan ke manifest.xml")
PY
    then :; else echo "  [FAIL] gagal edit manifest.xml - tambahkan manual dari vendor/etc/vintf/face_aidl_fragment.xml"; fi
else
    echo "  [WARN] $MANIFEST tidak ada - tambahkan manual dari vendor/etc/vintf/face_aidl_fragment.xml"
fi

# 4. SELinux file_contexts ---------------------------------------------------
# gen_file-contexts memberi label hal_allocator_default_exec ke file baru yang
# tidak punya entri -> service ditolak. Pastikan entri ada.
ctx_line_plain="bin/hw/$HAL_NAME"
add_ctx() {
    local f="$1" esc
    [[ -f "$f" ]] || return 0
    if tr -d '\\' < "$f" | grep -qF "$ctx_line_plain "; then
        echo "  [ok] file_contexts sudah punya entri: ${f#"$ROM_TREE"/}"
    else
        esc="$(sed -n '1p' "$HERE/config/vendor_file_contexts.add")"
        printf '%s\n' "$esc" >> "$f"
        echo "  [ok] file_contexts ditambah: ${f#"$ROM_TREE"/}"
    fi
}
add_ctx "$ROM_TREE/config/vendor_file_contexts"
add_ctx "$VENDOR/etc/selinux/vendor_file_contexts"
[[ -f "$ROM_TREE/config/vendor_file_contexts" || -f "$VENDOR/etc/selinux/vendor_file_contexts" ]] || \
    echo "  [WARN] tidak ada vendor_file_contexts - tambahkan baris dari config/vendor_file_contexts.add"

echo
echo "Selesai. Belum ada repack/flash - repack vendor dengan tool-mu lalu flash."
echo "Setelah boot: logcat -s FaceHalHW ; dumpsys face"
