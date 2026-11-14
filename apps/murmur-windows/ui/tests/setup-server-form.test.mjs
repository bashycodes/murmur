// Step 2's "Your server" panel: the address and token show straight away
// (no "I already have a server" fold), and "set up a server" stays.
import { test } from "node:test";
import assert from "node:assert/strict";
import { TEMPLATE } from "../dist/onboarding/templates.js";
import { SERVER_HELP } from "../dist/features/models.js";

/** The opening tag of the element with this id. */
const tagOf = (id) => TEMPLATE.match(new RegExp(`<[a-z]+[^>]*\\bid="${id}"[^>]*>`))?.[0];

test("the address and token fields are open by default", () => {
  const fields = tagOf("ob_server_fields");
  assert.ok(fields, "ob_server_fields is in the template");
  assert.doesNotMatch(fields, /\bhidden\b/);
  assert.ok(tagOf("ob_host") && tagOf("ob_token") && tagOf("ob_test_conn"));
});

test("no fold button sits in front of them", () => {
  assert.equal(tagOf("ob_server_more"), undefined);
  assert.doesNotMatch(TEMPLATE, /I already have a server/);
});

test("the set up a server link is kept", () => {
  assert.match(TEMPLATE, /set up a server →/);
  assert.ok(TEMPLATE.includes(SERVER_HELP));
});
