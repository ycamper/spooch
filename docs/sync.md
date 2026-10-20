# Syncing captures

Captures sync between the devices signed in to one account. Local-only
use is untouched: syncing is off until it's switched on, and nothing
leaves the machine before that. Decided on 2026-10-18: accounts
are made in the app, everything syncs once switched on, deleting keeps a
capture in the account's trash for 30 days, and there's no end-to-end
encryption ("they're just screenshots").

## Service (#152)

The API is in `service/README.md` (Accounts, Syncing captures).

- Accounts: name and password (argon2id), made in the app with the
  sign-up code the service is set with; signing in gives the device a
  token as before. Failed sign-ins lock a name out for 15 minutes.
- A capture is three parts, the document (`document.json`), the full image
  (`source.png`) and the thumbnail (`thumb.png`), uploaded straight to the
  bucket under `captures/<owner>/<id>/<version>/<part>`.
- One counter numbers every change and every version. A device asks what
  changed after the last number it saw; each change comes with a
  thumbnail URL, so previews need no more requests.
- A new version names the version it's based on. Two devices editing one
  capture can't silently overwrite each other: the second gets a conflict,
  pulls, and tries again.
- Deleting trashes a capture for 30 days (restorable), then purges it,
  leaving a `gone` record so every device drops it. `?forever=true`
  purges at once. A quota per account (10 GB by default).

## App (#153, #154)

- Signing in: the tray's Create account... and Sign in... ask for a name,
  password (and the sign-up code), and keep the device token in the OS
  keychain, as sharing does now; Set password... for an account made by
  hand.
- A Sync captures switch in the tray, off by default. When on, a
  background thread:
  - pushes each capture kept in Recents (new, or changed since its last
    push) as a new version, and pulls what changed since the last number
    it saw, every minute and whenever a capture is kept;
  - writes a capture another device made into Recents as a stub: its
    thumbnail and its sync note, no document or image yet.
- The gallery shows stubs like any capture, from their thumbnails, with a
  cloud badge. Opening one downloads its document and image first, then
  opens it as usual.
- Each Recents folder keeps a `sync.txt`: the capture's sync id, the
  version it's at, and the change number of its document as pushed, so a
  device knows what's new on either side.
- Deleting in the gallery trashes the capture in the account too; local
  pruning (30 days or 2 GB) only drops the local copy, which comes back as
  a stub.
- On a conflict the newer edit wins: the app pulls the other device's
  version and, if its own edit is newer, pushes again on top of it.
