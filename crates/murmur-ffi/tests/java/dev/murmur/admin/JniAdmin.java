package dev.murmur.admin;

// Single-file source launch: `java -Djava.library.path=DIR JniAdmin.java <cfgJson> <cfgJsonTemplate>`.
// Mirrors apps/murmur-android/JNI_CONTRACT.md "Admin API": static natives on
// dev.murmur.admin.NativeAdmin. Prints one answer per line (never the secret).
public class JniAdmin {
    public static void main(String[] args) {
        System.loadLibrary("murmur_ffi");
        String cfg = args[0];
        String template = args[1]; // "TOKEN" is replaced by the new key's secret
        System.out.println(NativeAdmin.nativeAdminWhoami(cfg));
        System.out.println(NativeAdmin.nativeAdminListKeys(cfg));
        String created = NativeAdmin.nativeAdminCreateKey(cfg, " phone ", false);
        String secret = field(created, "secret");
        System.out.println(created.replace(secret, "<SECRET>") + " LEN " + secret.length());
        String phone = template.replace("TOKEN", secret);
        System.out.println(NativeAdmin.nativeAdminWhoami(phone));
        System.out.println(NativeAdmin.nativeAdminListKeys(phone));
        System.out.println(NativeAdmin.nativeAdminRevokeKey(cfg, field(created, "id")));
        System.out.println(NativeAdmin.nativeAdminWhoami(phone));
        System.out.println(NativeAdmin.nativeAdminWhoami(null));
        System.out.println(NativeAdmin.nativeAdminCreateKey(cfg, null, true));
    }

    /** First `"name":"value"` in a JSON string (enough for this test). */
    static String field(String json, String name) {
        String k = "\"" + name + "\":\"";
        int i = json.indexOf(k);
        if (i < 0) throw new RuntimeException("no " + name + " in " + json);
        int s = i + k.length();
        return json.substring(s, json.indexOf('"', s));
    }
}

class NativeAdmin {
    static native String nativeAdminWhoami(String serverConfigJson);
    static native String nativeAdminListKeys(String serverConfigJson);
    static native String nativeAdminCreateKey(String serverConfigJson, String name, boolean admin);
    static native String nativeAdminRevokeKey(String serverConfigJson, String id);
}
