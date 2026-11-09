// The single owner of the engine settings (dev_settings.json) on this page:
// the ONLY caller of `dev_set`. `dev_set` takes the whole form, so a page
// saves a patch of the fields it shows and the store lays it over the last
// saved form; one page's save never sends another page's unsaved edits.
// The saved token never comes back (S18): `token: ""` keeps it, a typed
// token replaces it, `token_saved: false` removes it.

import { invoke } from "./ipc.js";
import type { DevForm, DevView } from "./types.js";
import { views } from "./views.js";

/** The last saved form, with the cloud fields the OpenRouter page owns removed. */
function savedForm(): DevForm {
  const base = views.dev?.form;
  if (!base) throw new Error("engine settings not loaded yet");
  const f: DevForm = { ...base, token: "" };
  // D28: the cloud settings are the OpenRouter page's (`dev_or_*`); never sent from here.
  delete (f as unknown as Record<string, unknown>).cloud;
  delete (f as unknown as Record<string, unknown>).allow_cloud;
  return f;
}

export const devStore = {
  /** The form `patch` would save (for `dev_validate` / `dev_test_connection`). */
  form(patch: Partial<DevForm> = {}): DevForm {
    return { ...savedForm(), ...patch };
  },

  /** Save `patch` over the last saved form and publish the new view.
   *  Throws the backend's FieldError[] or message. */
  async save(patch: Partial<DevForm>): Promise<DevView> {
    const v = await invoke<DevView>("dev_set", { form: devStore.form(patch) });
    views.setDev(v);
    return v;
  },
};
