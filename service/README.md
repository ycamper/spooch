# spoochd

Spooch's sharing and sync service: the app uploads a capture and gets a
link that works anywhere, and an account's captures sync between its
devices. Accounts are made in the app with a sign-up code, or by hand.

## Run it locally

From this folder (in WSL, or anywhere with Docker). Storage is a dev
bucket on DigitalOcean Spaces: copy `.env.example` to `.env` and fill in
its keys (`.env` is git-ignored).

```
docker compose up --build
docker compose exec spoochd /spoochd user add alex
docker compose exec spoochd /spoochd token add alex laptop
```

The last command prints a device token once; put it in the app.
`/spoochd token list` and `token revoke ID` manage them. The API is on
http://localhost:8080.

Tests need only Go. The store tests also run against Postgres when
`SPOOCH_TEST_DATABASE_URL` names an empty database (CI sets one):

```
docker run --rm -v "$PWD":/src -w /src golang:1.23 go test ./...
```

Production runs on one server behind nginx; `deploy/` has the systemd unit,
the nginx site and the scripts that install and update it.

## How a share works

1. The app makes the link itself: a 10-character id and a 22-character
   secret (base62), and puts `PUBLIC_URL/<id><secret>` on the clipboard.
2. `POST /v1/shares` with `{id, secret_hash, size, type, expiry_days}`
   (`secret_hash` is the lowercase hex SHA-256 of the secret;
   `expiry_days` 0 for 30 days, at most 90). The reply has a presigned
   `upload_url`, signed for exactly that `Content-Type` and size. Shares
   count toward the account's quota like synced captures (507 when it's
   full), and an account can make 100 a day (429 after that).
3. The app PUTs the image there, then `POST /v1/shares/<id>/finalize`.
   The service checks the size and that it's really a PNG or JPEG, then
   copies it to where links read it, and only then does the link work.

Every API call carries `Authorization: Bearer <token>`. `GET /v1/me`
names the account and gives `links`, the `PUBLIC_URL` links start with.
Request bodies are strict JSON: unknown fields are refused.

## Who can see a share

- **Anyone with the link**: `"public": true`, the default.
- **Specific accounts**: `"recipients": ["dana", "sam"]`, by account name.
  They see it in Spooch through `GET /v1/shared-with-me` and
  `GET /v1/shares/<id>/image`. Opened in a browser, an account-only link
  says to open Spooch; a web sign-in comes later.
- **For N days**: `"expiry_days": N` (0 for 30 days, at most 90).

All three can be set when sharing, and changed later with
`PATCH /v1/shares/<id>`, sending only the fields to change.
`GET /v1/shares` lists your live shares with who they're shared with.
To stop sharing, set `"public": false` and `"recipients": []`, or
`DELETE /v1/shares/<id>` to remove the image altogether.

`/<id><secret>` is a page with the image and a chat preview;
`/<id><secret>.png` (or `.jpg`) is the image itself. The service keeps
only hashes of tokens and secrets, so a leaked database opens nothing,
and nginx logs a link's id without its secret. Expired and
never-finished shares are deleted every ten minutes.

## Accounts

People make their own accounts in Spooch, if they have the sign-up code
the service is set with (`SPOOCH_SIGNUP_CODE`; without one, sign-ups are
closed). Accounts made by hand have no password until they set one.

- `POST /v1/accounts` with `{name, password, code, device}` makes one and
  signs that device in: the reply has its `token` (shown once) and its
  `device` id. A name is 3 to 32 of `a-z 0-9 . _ -`; a password, 10 to 256
  characters.
- `POST /v1/sign-in` with `{name, password, device}` signs another device
  in the same way. Ten wrong passwords in 15 minutes lock the name out
  until they pass. A wrong name takes as long to refuse as a wrong
  password.
- `PUT /v1/password` with `{current, password}` sets or changes it
  (`current` only when there is one).
- `GET /v1/devices` lists the account's devices; `DELETE /v1/devices/<id>`
  signs one out, this one or another.

Passwords are kept as argon2id hashes.

## Syncing captures

An account's captures sync between its devices. Each is a document, a
full image and a thumbnail, all uploaded straight to the bucket, under
an id the app picks (16 base62 characters). Every change gets a number
from one counter, and each version is numbered from it too.

1. `PUT /v1/captures/<id>` with `{base, taken, doc_size, source_size,
   thumb_size}` begins a version on top of `base` (0 for a new capture).
   The reply has its `version` and a presigned PUT in `uploads` for each
   part (`doc` is JSON, `source` and `thumb` PNG, as `types` says). If
   another device changed it meanwhile, 409 with the `version` it's at.
2. The app uploads the three parts, then `POST /v1/captures/<id>/finish`
   with `{version}`. The service checks each part's size and how it
   starts, makes the version live and removes the one before.
3. `GET /v1/captures?since=<seq>` lists what changed after `seq`, oldest
   first: each capture's `version`, `seq`, `taken`, `size`, `trashed` or
   `gone`, and a `thumb_url` that works for an hour, for previews.
   `next` is where to carry on from, and `more` says there is more.
4. `GET /v1/captures/<id>/<doc|source|thumb>` is a download URL for a part
   of the live version, for when it's opened.

`DELETE /v1/captures/<id>` puts it in the trash, where it can't change;
`POST /v1/captures/<id>/restore` takes it back out. After 30 days in the
trash it's deleted for good, or at once with `?forever=true`; its record
stays, marked `gone`, so every device learns it went. Each account may
keep `SPOOCH_QUOTA_GB` of captures, the trash included. Uploads not
finished within the hour are cleared away.

## Configuration

| Variable | |
| --- | --- |
| `SPOOCH_DATABASE_URL` | Postgres URL (required) |
| `SPOOCH_PUBLIC_URL` | Where links point, like `https://img.spooch.app` |
| `SPOOCH_ADDR` | Listen address, `:8080` |
| `SPOOCH_S3_ENDPOINT` | Bucket host, like `nyc3.digitaloceanspaces.com` |
| `SPOOCH_S3_PUBLIC_ENDPOINT` | Host the app uploads to, if different |
| `SPOOCH_S3_REGION` | Like `nyc3` |
| `SPOOCH_S3_BUCKET` | `spooch` |
| `SPOOCH_S3_KEY`, `SPOOCH_S3_SECRET` | Access keys (required); keep them out of git |
| `SPOOCH_S3_SECURE` | `true` for HTTPS (default) |
| `SPOOCH_SIGNUP_CODE` | The code creating an account takes; none closes sign-ups |
| `SPOOCH_QUOTA_GB` | GB of synced captures per account, `10` |
