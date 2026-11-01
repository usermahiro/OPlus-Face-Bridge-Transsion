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
