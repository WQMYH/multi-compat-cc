# DSH application integration

The DSH application page reuses CC's existing provider editor, presets,
universal providers and managed accounts. Only providers explicitly added to
DSH appear in its `dsh` database namespace; it does not mirror the Codex list
or require a second account login. CLI detection and installation use CC's
application management entry, not a manual executable-path form.

## Native configuration and model selection

- Ordinary API-key providers use DSH's native OpenAI Chat, Responses or
  Anthropic protocol, as configured in the existing provider editor.
- Apply merges providers and model options into existing desktop/web patches
  and updates the native default. Providers coexist. DSH's own model selector
  supplies aggregation; CC does not create another model pool or queue.
- Native writes use DSH's lock protocol, preimage checks, private atomic writes
  and readback. Existing history and unrelated settings are retained. A
  nonempty global overlay or custom credential-store location is rejected
  before any write rather than silently overwritten.
- Existing running turns and per-session overrides are not forcibly changed.
  Saved defaults and a catalog read are not proof of successful inference.
- Detected Desktop/Web defaults enter the existing CC provider cards. They use
  the same select, in-use state and exit-to-direct confirmation, not a separate
  restore area. Selection restores each profile's own native choice; it does
  not copy Desktop's provider or credentials to Web.
- Native reference IDs stay stable across credential-store revisions. Balance
  queries retain their separate, revision-checked identity. Native credentials
  remain managed by DSH and are not converted into CC API-key providers.

## Existing CC-managed Codex accounts

DSH uses native `openai-responses` on CC's existing proxy listener. Forwarding
reuses CC's normal Responses path, CodexOAuth account manager, streaming
response handling and usage logging; there is no forced Anthropic conversion.
Usage belongs to `dsh`; Claude/Codex's current provider and routing state are
not changed. There is no new forwarding server, account store or login flow.

The DSH row must reference an explicit managed account and a configured model.
OAuth tokens remain with CC's existing account manager. DSH stores only the
local proxy credential through its native credential references. Missing
accounts, providers and models are rejected without selecting another target.

Earlier native/provider-pinned configurations are not automatically replayed.
Changing the selected provider does not resubmit any message or resolve an
unknown historical request.

## Verification

Use `CC_SWITCH_TEST_HOME` outside the real CC profile, as required by
`CONTRIBUTING.md`. The `source10_dsh_` Rust regressions cover native protocol
selection, fixed-provider shared forwarding, local header authentication,
existing account conversion and non-dispatching failures. These are synthetic
checks, not real model acceptance.

The `source19_` regressions cover common native-reference selection, per-profile
default retention through CC direct/routing, and stable selection versus
revision-bound balance identities. Native-library checks under `compat/dsh`
use synthetic homes and balances only. They do not authenticate a real account
or invoke a model.

The optional native settings bridge still uses revision-checked
`settings/describe` / `settings/mutate` and `session/modelCatalog`. Its checks
are `pnpm test:dsh-control`; generate its bundled resource with
`scripts/build-dsh-native.mjs`. Unknown writes must be reconciled before
another edit. Never read back or log credential values to diagnose a failure.
