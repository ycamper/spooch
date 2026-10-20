CREATE TABLE IF NOT EXISTS users (
    id      BIGSERIAL PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS devices (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users (id),
    label      TEXT NOT NULL,
    -- SHA-256 of the token; the token itself is shown once and not kept.
    token_hash TEXT NOT NULL UNIQUE,
    created    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used  TIMESTAMPTZ,
    revoked    BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS shares (
    id          TEXT PRIMARY KEY,
    owner       BIGINT NOT NULL REFERENCES users (id),
    -- SHA-256 of the link's secret, so a leaked table gives no working links.
    secret_hash TEXT NOT NULL,
    size        BIGINT NOT NULL,
    type        TEXT NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires     TIMESTAMPTZ,
    state       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS shares_owner ON shares (owner, created DESC);

-- Whether anyone with the link may view a share, or only its recipients.
ALTER TABLE shares ADD COLUMN IF NOT EXISTS public BOOLEAN NOT NULL DEFAULT true;

-- What the sweeper looks for.
CREATE INDEX IF NOT EXISTS shares_sweep ON shares (state, expires, created)
    WHERE state <> 'deleted';

-- The accounts a share is shared with.
CREATE TABLE IF NOT EXISTS share_recipients (
    share_id TEXT NOT NULL REFERENCES shares (id),
    user_id  BIGINT NOT NULL REFERENCES users (id),
    PRIMARY KEY (share_id, user_id)
);

CREATE INDEX IF NOT EXISTS share_recipients_user ON share_recipients (user_id);

-- Accounts made in the app sign in with a password; ones made by hand
-- have none until they set one.
ALTER TABLE users ADD COLUMN IF NOT EXISTS password_hash TEXT;

-- One counter orders every capture change and numbers every version.
CREATE SEQUENCE IF NOT EXISTS capture_changes;

-- The captures an account syncs. A version's parts live at
-- captures/<owner>/<id>/<version>/<part>.
CREATE TABLE IF NOT EXISTS captures (
    owner          BIGINT NOT NULL REFERENCES users (id),
    id             TEXT NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,
    seq            BIGINT NOT NULL DEFAULT 0,
    taken          TIMESTAMPTZ NOT NULL,
    updated        TIMESTAMPTZ NOT NULL,
    doc_size       BIGINT NOT NULL DEFAULT 0,
    source_size    BIGINT NOT NULL DEFAULT 0,
    thumb_size     BIGINT NOT NULL DEFAULT 0,
    pending        BIGINT,
    pending_doc    BIGINT,
    pending_source BIGINT,
    pending_thumb  BIGINT,
    begun          TIMESTAMPTZ,
    trashed        TIMESTAMPTZ,
    gone           BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (owner, id)
);

CREATE INDEX IF NOT EXISTS captures_changes ON captures (owner, seq);
CREATE INDEX IF NOT EXISTS captures_trash ON captures (trashed)
    WHERE trashed IS NOT NULL AND NOT gone;
CREATE INDEX IF NOT EXISTS captures_begun ON captures (begun) WHERE begun IS NOT NULL;
