# DSH application integration

The DSH application page reuses CC's existing provider editor, presets,
universal providers and managed accounts. Only providers explicitly added to
DSH appear in its `dsh` database namespace; it does not mirror the Codex list
or require a second account login. Existing Desktop/Web providers and model
catalogs are observed separately without importing their credentials. CC-owned
projections are not imported again as native cards.
Built-in DeepSeek adapters alone do not create account cards: unselected
entries require native credential-presence metadata. Explicit defaults remain
visible for repair; presence is not a claim of valid authorization or balance.
CLI detection and installation use CC's
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
- Native provider cards reuse CC's model menu. Selecting a model changes only
  that card's Desktop or Web default through the shared guarded write engine;
  the model must still exist in the freshly observed native catalog. Native
  credentials are not copied or made routable by importing their references.
  An offline catalog is not a live account entitlement or inference check.

## Native history

DSH's compressed session logs enter the existing CC session browser and
transcript cache. The adapter reads committed user/assistant messages, not
stream deltas or private reasoning. Oversized or corrupt sources are skipped,
not reported as successfully parsed partial transcripts. Deletion is available
only through the shared confirmation flow and is confined to the native
session directory whose header identity matches. No resume command is invented.

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

Managed model catalogs retain each model's advertised reasoning levels and
default separately. An existing ID-only catalog is enriched without adding
unselected models or replacing explicit capability overrides. Reapply the
current DSH route to publish the corrected capabilities to Desktop/Web.
DSH's supported thinking levels are preserved; Codex's delegated `ultra` mode
is not mapped to `max` or offered as an unsupported DSH level.

Earlier native/provider-pinned configurations are not automatically replayed.
Changing the selected provider does not resubmit any message or resolve an
unknown historical request.

## Verification

Use `CC_SWITCH_TEST_HOME` outside the real CC profile, as required by
`CONTRIBUTING.md`. The `source11_dsh_` Rust regressions cover common provider
CRUD, existing-account binding, Responses forwarding and non-dispatching
failures. These are synthetic checks, not real model acceptance.

The `source19_` regressions cover common native-reference selection, per-profile
default retention through CC direct/routing, and stable selection versus
revision-bound balance identities. Native-library checks under `compat/dsh`
use synthetic homes and balances only. They do not authenticate a real account
or invoke a model.

Run native provider fixtures with `cargo test --lib dsh_direct::tests::` and
the pure editor contract with `cargo test --lib native_provider_config::tests::`.
The installed-library fixtures use `node --test
compat/dsh/native-defaults.test.mjs compat/dsh/native-balance.test.mjs`;
they require compatible installed DSH libraries but use synthetic homes.
The native inventory check is `node --test compat/dsh/native-inventory.test.mjs`
with `DSH_TEST_CLI` pointing to the installed native CLI entry. The `source2_`
tests cover catalog-checked profile-local selection and common publication;
`session_manager::providers::dsh::tests::` covers native history with temporary
compressed fixtures. None of these tests calls a model or modifies real profiles.

There is no separate Plan login or token store. The removed legacy Plan
registration is never resumed or migrated; any existing registration files
are left untouched. Unknown writes must be reconciled before another edit.
Never read back or log credential values to diagnose a failure.
