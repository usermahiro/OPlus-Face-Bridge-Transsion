# Face HAL (AIDL) - paket implementasi ROM @VERSION@

Isi paket:

```
vendor/bin/hw/@HAL_NAME@     binary HAL (aarch64, PIE)
vendor/etc/init/@RC_NAME@    init rc (service @SERVICE_NAME@)
vendor/etc/vintf/face_aidl_fragment.xml   deklarasi VINTF AIDL face v3
config/vendor_file_contexts.add           baris label SELinux
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

## Preview enroll: satu langkah wajib setelah flash

HAL yang jalan dari `/vendor/bin/hw` tidak bisa memuat `libgui.so`. Linker memberi proses itu
class `vendor`: search path hanya `/odm/lib64`, `/vendor/lib64`, `/vendor/lib64/hw`,
`/vendor/lib64/egl`, dan pustaka sistem hanya yang ada di daftar LLNDK. `libgui.so` tidak ada di
situ, jadi layer preview tidak bisa dibuat dari sana.

Binary yang dieksekusi dari bawah `/data/local/tmp` masuk class `unrestricted` dan bisa memuat
`libgui.so`. Karena itu HAL menjalankan ulang salinan dirinya dari
`/data/local/tmp/facehal/`. Salinan itu harus sudah ada dan berlabel benar; HAL tidak bisa
membuatnya sendiri karena jalan sebagai `system` sedangkan `/data/local/tmp` milik `shell`.

Setelah flash, dari PC:

```bash
adb push vendor/bin/hw/@HAL_NAME@ /data/local/tmp/facehal/@HAL_NAME@
adb shell su -c 'chmod 0755 /data/local/tmp/facehal/@HAL_NAME@ && chcon u:object_r:hal_face_default_exec:s0 /data/local/tmp/facehal/@HAL_NAME@'
adb shell su -c 'stop @SERVICE_NAME@ && start @SERVICE_NAME@'
```

Atau kalau `script.sh` ada di HP: `sh script.sh persist`.

Sekali saja. Salinannya ada di `/data`, bukan tmpfs, jadi bertahan setelah reboot. Ulangi
kalau binary HAL-nya diganti.

Cek: `adb logcat -d -s FaceHalHW | grep reexec` harus berbunyi
`reexec: menjalankan /data/local/tmp/facehal/@HAL_NAME@ supaya libgui bisa dimuat`.
Kalau barisnya berbunyi `tidak ada` atau `tidak bisa dieksekusi`, enroll dan buka kunci tetap
jalan normal, hanya preview yang hilang.

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
