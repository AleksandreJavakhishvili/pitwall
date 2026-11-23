# Setup by an agent

Users can ask an agent (the Race Engineer, or any coding agent in Pitwall)
to set up your extension. The agent runs `pitwall extension status <id>`,
which reads your card as data: the same checklist the user sees, with
`next`, the one step to take. It loops `status`, then `next`, then
`status` again. Every change it asks for (install, turn on, a setup
action, your service's confirm, its first run, a plain setting) shows the
user an approval in Pitwall listing exactly what will run, and runs only
when they approve. A secret is never passed through the agent: it opens
Settings at your secret's field and the user enters it there. A first
run's link goes only to the user's browser.

What this means for you:

- **Labels and hints are read aloud.** A setting's `label` and `hint`, an
  action's `label` and `description`, your service's `label` and first
  run's `label` reach the agent and the approval. Write them for a person
  who hasn't seen your card.
- **Mark setup actions** (`"setup": true`) that a user must run to be
  set up: the agent runs them in order. Others stay under More.
- **Say why it stops.** An adapter's `status` reason (signed out, the
  server unreachable) is what the agent tells the user. Your Log is
  readable by the agent too (`pitwall extension log <id>`), secrets taken
  out, so keep stderr useful.
- **A version requirement** on your service (`version.requires`) lets
  the agent offer `service <id> update` (your install action) when the
  one installed is too old.

The full rules are in [`docs/spec/extensions.md`](../spec/extensions.md),
"Setup by an agent".
