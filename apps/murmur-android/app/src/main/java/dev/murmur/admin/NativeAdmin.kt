package dev.murmur.admin

import dev.murmur.core.NativeMurmurCore

/**
 * D39 admin API natives (`crates/murmur-ffi/src/jni_admin.rs`, feature `admin`). Contract:
 * apps/murmur-android/JNI_CONTRACT.md "Admin API". Only the `hybrid` / `remote` libraries export
 * them; the `local` one has none, which [NativeAdminApi] reads as "not in this build".
 * All but [nativeTeamDictionaryCached] (file I/O only) are **blocking** (one HTTP request, ≤ 15 s) and answer the JSON envelope
 * `{"ok":true,"value":…}` / `{"ok":false,"code":…,"message":…}`. Never null.
 */
object NativeAdmin {
    /** The library is `libmurmur_ffi.so`, loaded (once) by [NativeMurmurCore.loadLibrary]. */
    fun loaded(): Boolean = NativeMurmurCore.loadLibrary() is NativeMurmurCore.LibraryState.Loaded

    @JvmStatic external fun nativeAdminWhoami(serverConfigJson: String): String
    @JvmStatic external fun nativeAdminListKeys(serverConfigJson: String): String
    @JvmStatic external fun nativeAdminCreateKey(serverConfigJson: String, name: String, admin: Boolean): String
    @JvmStatic external fun nativeAdminRevokeKey(serverConfigJson: String, id: String): String

    // Team dictionary ([TeamDictionaryApi]); `cachePath` = `filesDir/team-dictionary.json`.
    @JvmStatic external fun nativeTeamDictionaryCached(serverConfigJson: String, cachePath: String): String
    @JvmStatic external fun nativeTeamDictionaryRefresh(serverConfigJson: String, cachePath: String): String
    @JvmStatic external fun nativeTeamDictionaryAdd(serverConfigJson: String, cachePath: String, word: String, replacement: String): String
    @JvmStatic external fun nativeTeamDictionaryRemove(serverConfigJson: String, cachePath: String, word: String): String
}
