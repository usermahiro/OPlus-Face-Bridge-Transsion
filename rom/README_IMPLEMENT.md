# Face HAL (AIDL) - paket implementasi ROM @VERSION@

Isi paket:

```
vendor/bin/hw/@HAL_NAME@     binary HAL (aarch64, PIE)
vendor/etc/init/@RC_NAME@    init rc (service @SERVICE_NAME@)
vendor/etc/init/face-preview-prep.rc      init action: salinan untuk re-exec HAL
vendor/etc/vintf/face_aidl_fragment.xml   deklarasi VINTF AIDL face v3
config/vendor_file_contexts.add           baris label SELinux untuk binary
config/plat_file_contexts.add             baris label SELinux untuk salinan di /data
install_to_rom.sh                         pasang semuanya ke ROM tree
SHA256SUMS
```

## Pasang ke ROM tree (unpacked)

```bash
unzip face-hal-rom-@VERSION@.zip && cd face-hal-rom-@VERSION@
ROM_TREE=/path/ke/erofs ./install_to_rom.sh
```

`ROM_TREE` = folder yang berisi `vendor/` (dan idealnya `config/vendor_file_contexts`).
Script ini hanya menulis file: tidak repack, tidak flash.

Yang dikerjakan script:
1. Salin binary ke `/vendor/bin/hw/@HAL_NAME@` (0755).
2. Tulis rc `@RC_NAME@` dan peringatkan kalau ada rc lain yang masih start service face (dobel service).
3. Sisipkan entri AIDL `android.hardware.biometrics.face` v3 ke `vendor/etc/vintf/manifest.xml`
   (kalau belum ada). Tanpa ini `FaceService` tidak pernah menemukan HAL (`waitForDeclaredService`).
4. Tambah label `hal_face_default_exec` ke `vendor_file_contexts` bila belum ada.

## Preview enroll: salinan di /data/local/tmp

HAL yang jalan dari `/vendor/bin/hw` tidak bisa memuat `libgui.so`. Linker memberi proses itu
class `vendor`: search path hanya `/odm/lib64`, `/vendor/lib64`, `/vendor/lib64/hw`,
`/vendor/lib64/egl`, dan pustaka sistem hanya yang ada di daftar LLNDK. `libgui.so` tidak ada di
situ, jadi layer preview tidak bisa dibuat dari sana.

Binary yang dieksekusi dari bawah `/data/local/tmp` masuk class `unrestricted` dan bisa memuat
`libgui.so`. Karena itu HAL menjalankan ulang salinan dirinya dari
`/data/local/tmp/facehal/`.

Yang harus ada di ROM supaya itu terjadi tanpa langkah manual:

| file | isi |
|---|---|
| `vendor/etc/init/face-preview-prep.rc` | `on post-fs-data`: `mkdir`, `copy` dua kali, `chmod`, `chown`, `restorecon` |
| `system/etc/selinux/plat_file_contexts` | dua baris dari `config/plat_file_contexts.add` |

`install_to_rom.sh` memasang keduanya. Label harus dari `plat_file_contexts`, bukan
`vendor_file_contexts`: file itu tidak boleh memberi label pada path `/data`. Tanpa entrinya
salinan tetap `shell_data_file`, `restorecon` tidak mengubahnya, dan exec ditolak.

Cek sekali setelah flash:

```bash
adb logcat -d -s FaceHalHW | grep reexec
```

Harus ada `reexec: menjalankan /data/local/tmp/facehal/... supaya libgui bisa dimuat`. Kalau
barisnya `tidak ada` atau `tidak bisa dieksekusi`, enroll dan buka kunci tetap normal, hanya
preview yang hilang, dan penyebabnya label atau `restorecon`.

Kalau ROM tree system-nya tidak ikut di-unpack, tambahkan entri itu sendiri, atau sekali saja
dari PC:

```bash
adb push vendor/bin/hw/@HAL_NAME@ /data/local/tmp/facehal/
adb shell su -c 'chmod 0755 /data/local/tmp/facehal/@HAL_NAME@ && chcon u:object_r:hal_face_default_exec:s0 /data/local/tmp/facehal/@HAL_NAME@'
```

## Setelah flash - cek cepat

```bash
adb shell getprop init.svc.@SERVICE_NAME@        # running
adb shell service list | grep biometrics.face     # IFace/default terdaftar
adb logcat -s FaceHalHW                          # log HAL
adb shell setprop persist.sys.facehal.verbose 1  # log detail
adb shell logcat -G 32M                          # buffer besar sebelum tes enroll
adb shell am start -a android.settings.FACE_ENROLL
```

Service restart terus tiap beberapa detik = HAL gagal register (cek avc denial `servicemanager`/`hwservicemanager`
dan apakah entri VINTF benar-benar ada di manifest efektif).

## Catatan

- Binary ini memanggil `libstfaceunlockppl.so`, `libcamera2ndk_vendor.so`, `libmediandk.so` dari partisi vendor
  ROM-mu. Library itu TIDAK ikut di paket.
- Offset patch lisensi/GOT di source (`src/stface.rs`, `src/surface_bridge.rs`) spesifik build X6739.
  Di build vendor lain harus dicek ulang.
