package dev.flowclone.admin

import dev.flowclone.core.NativeFlowCore

/**
 * D39 admin API natives (`crates/flow-ffi/src/jni_admin.rs`, feature `admin`). Contract:
 * apps/flow-android/JNI_CONTRACT.md "Admin API". Only the `hybrid` / `remote` libraries export
 * them; the `local` one has none, which [NativeAdminApi] reads as "not in this build".
 * All four are **blocking** (one HTTP request, ≤ 15 s) and answer the JSON envelope
 * `{"ok":true,"value":…}` / `{"ok":false,"code":…,"message":…}`. Never null.
 */
object NativeAdmin {
    /** The library is `libflow_ffi.so`, loaded (once) by [NativeFlowCore.loadLibrary]. */
    fun loaded(): Boolean = NativeFlowCore.loadLibrary() is NativeFlowCore.LibraryState.Loaded

    @JvmStatic external fun nativeAdminWhoami(serverConfigJson: String): String
    @JvmStatic external fun nativeAdminListKeys(serverConfigJson: String): String
    @JvmStatic external fun nativeAdminCreateKey(serverConfigJson: String, name: String, admin: Boolean): String
    @JvmStatic external fun nativeAdminRevokeKey(serverConfigJson: String, id: String): String
}
