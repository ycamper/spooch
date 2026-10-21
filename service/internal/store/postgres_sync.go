package store

import (
	"context"
	"errors"
	"time"

	"github.com/jackc/pgx/v5"
)

func (p *Postgres) AddAccount(ctx context.Context, name, passwordHash string) (User, error) {
	u := User{Name: name}
	err := p.pool.QueryRow(ctx,
		`INSERT INTO users (name, password_hash) VALUES ($1, $2) RETURNING id`,
		name, passwordHash).Scan(&u.ID)
	return u, exists(err)
}

func (p *Postgres) Password(ctx context.Context, name string) (User, string, error) {
	u := User{Name: name}
	var hash *string
	err := p.pool.QueryRow(ctx, `SELECT id, password_hash FROM users WHERE name = $1`, name).
		Scan(&u.ID, &hash)
	if err != nil {
		return User{}, "", notFound(err)
	}
	if hash == nil {
		return u, "", nil
	}
	return u, *hash, nil
}

func (p *Postgres) SetPassword(ctx context.Context, user int64, hash string) error {
	return changed(p.pool.Exec(ctx, `UPDATE users SET password_hash = $2 WHERE id = $1`, user, hash))
}

func (p *Postgres) UserDevices(ctx context.Context, user int64) ([]Device, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT d.id, u.name, d.label, d.created, d.last_used, d.revoked
		 FROM devices d JOIN users u ON u.id = d.user_id
		 WHERE d.user_id = $1 AND NOT d.revoked ORDER BY d.id`, user)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Device, error) {
		var d Device
		err := r.Scan(&d.ID, &d.User, &d.Label, &d.Created, &d.LastUsed, &d.Revoked)
		return d, err
	})
}

func (p *Postgres) RevokeUserDevice(ctx context.Context, user, id int64) error {
	return changed(p.pool.Exec(ctx,
		`UPDATE devices SET revoked = true WHERE id = $1 AND user_id = $2 AND NOT revoked`, id, user))
}

const captureColumns = `id, owner, version, seq, taken, updated, doc_size, source_size, thumb_size,
	COALESCE(pending, 0), COALESCE(pending_doc, 0), COALESCE(pending_source, 0), COALESCE(pending_thumb, 0),
	begun, trashed, gone`

func scanCapture(r pgx.Row) (Capture, error) {
	var c Capture
	err := r.Scan(&c.ID, &c.Owner, &c.Version, &c.Seq, &c.Taken, &c.Updated,
		&c.Sizes.Doc, &c.Sizes.Source, &c.Sizes.Thumb,
		&c.Pending, &c.PendingSizes.Doc, &c.PendingSizes.Source, &c.PendingSizes.Thumb,
		&c.Begun, &c.Trashed, &c.Gone)
	return c, err
}

func (p *Postgres) BeginCapture(ctx context.Context, owner int64, id string, base int64, taken time.Time, sizes Sizes, now time.Time) (version, replaced int64, err error) {
	err = pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		c, err := scanCapture(tx.QueryRow(ctx,
			`SELECT `+captureColumns+` FROM captures WHERE owner = $1 AND id = $2 FOR UPDATE`, owner, id))
		switch {
		case errors.Is(err, pgx.ErrNoRows) && base != 0:
			return ErrConflict
		case errors.Is(err, pgx.ErrNoRows):
			_, err = tx.Exec(ctx,
				`INSERT INTO captures (owner, id, taken, updated) VALUES ($1, $2, $3, $4)`,
				owner, id, taken, now)
			// Another device made it meanwhile.
			if errors.Is(exists(err), ErrExists) {
				return ErrConflict
			} else if err != nil {
				return err
			}
		case err != nil:
			return err
		case c.Gone:
			return ErrGone
		case c.Trashed != nil:
			return ErrTrashed
		case c.Version != base:
			return ErrConflict
		}
		replaced = c.Pending
		return tx.QueryRow(ctx,
			`UPDATE captures SET pending = nextval('capture_changes'),
			   pending_doc = $3, pending_source = $4, pending_thumb = $5, begun = $6
			 WHERE owner = $1 AND id = $2 RETURNING pending`,
			owner, id, sizes.Doc, sizes.Source, sizes.Thumb, now).Scan(&version)
	})
	return version, replaced, err
}

func (p *Postgres) FinishCapture(ctx context.Context, owner int64, id string, version int64, now time.Time) (Capture, error) {
	var before Capture
	err := pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		var err error
		before, err = scanCapture(tx.QueryRow(ctx,
			`SELECT `+captureColumns+` FROM captures
			 WHERE owner = $1 AND id = $2 AND pending = $3 AND NOT gone AND trashed IS NULL
			 FOR UPDATE`, owner, id, version))
		if err != nil {
			return notFound(err)
		}
		_, err = tx.Exec(ctx,
			`UPDATE captures SET version = pending, doc_size = pending_doc,
			   source_size = pending_source, thumb_size = pending_thumb,
			   updated = $3, seq = nextval('capture_changes'),
			   pending = NULL, pending_doc = NULL, pending_source = NULL, pending_thumb = NULL,
			   begun = NULL
			 WHERE owner = $1 AND id = $2`, owner, id, now)
		return err
	})
	return before, err
}

func (p *Postgres) Capture(ctx context.Context, owner int64, id string) (Capture, error) {
	c, err := scanCapture(p.pool.QueryRow(ctx,
		`SELECT `+captureColumns+` FROM captures WHERE owner = $1 AND id = $2`, owner, id))
	return c, notFound(err)
}

func (p *Postgres) Changes(ctx context.Context, owner int64, since int64, limit int) ([]Capture, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT `+captureColumns+` FROM captures
		 WHERE owner = $1 AND seq > $2 AND version <> 0 ORDER BY seq LIMIT $3`, owner, since, limit)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Capture, error) { return scanCapture(r) })
}

func (p *Postgres) TrashCapture(ctx context.Context, owner int64, id string, now time.Time) error {
	return changed(p.pool.Exec(ctx,
		`UPDATE captures SET trashed = $3, seq = nextval('capture_changes')
		 WHERE owner = $1 AND id = $2 AND version <> 0 AND NOT gone AND trashed IS NULL`,
		owner, id, now))
}

func (p *Postgres) RestoreCapture(ctx context.Context, owner int64, id string) error {
	return changed(p.pool.Exec(ctx,
		`UPDATE captures SET trashed = NULL, seq = nextval('capture_changes')
		 WHERE owner = $1 AND id = $2 AND version <> 0 AND NOT gone AND trashed IS NOT NULL`,
		owner, id))
}

func (p *Postgres) PurgeCapture(ctx context.Context, owner int64, id string) (Capture, error) {
	var before Capture
	err := pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		var err error
		before, err = purge(ctx, tx, owner, id)
		return err
	})
	return before, err
}

// purge is PurgeCapture inside `tx`.
func purge(ctx context.Context, tx pgx.Tx, owner int64, id string) (Capture, error) {
	before, err := scanCapture(tx.QueryRow(ctx,
		`SELECT `+captureColumns+` FROM captures
		 WHERE owner = $1 AND id = $2 AND version <> 0 AND NOT gone FOR UPDATE`, owner, id))
	if err != nil {
		return Capture{}, notFound(err)
	}
	_, err = tx.Exec(ctx,
		`UPDATE captures SET gone = true, seq = nextval('capture_changes'),
		   pending = NULL, pending_doc = NULL, pending_source = NULL, pending_thumb = NULL, begun = NULL
		 WHERE owner = $1 AND id = $2`, owner, id)
	return before, err
}

func (p *Postgres) Usage(ctx context.Context, owner int64) (int64, error) {
	var total int64
	err := p.pool.QueryRow(ctx,
		`SELECT
		   (SELECT COALESCE(SUM(
		      CASE WHEN gone THEN 0 ELSE doc_size + source_size + thumb_size END
		      + COALESCE(pending_doc + pending_source + pending_thumb, 0)), 0)
		    FROM captures WHERE owner = $1)
		 + (SELECT COALESCE(SUM(size), 0)
		    FROM shares WHERE owner = $1 AND state IN ('live', 'pending'))`, owner).Scan(&total)
	return total, err
}

func (p *Postgres) SweepCaptures(ctx context.Context, trashedBefore, begunBefore time.Time, limit int) (purged, dropped []Capture, err error) {
	err = pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		rows, err := tx.Query(ctx,
			`SELECT owner, id FROM captures
			 WHERE trashed < $1 AND NOT gone ORDER BY trashed LIMIT $2 FOR UPDATE SKIP LOCKED`,
			trashedBefore, limit)
		if err != nil {
			return err
		}
		type key struct {
			owner int64
			id    string
		}
		old, err := pgx.CollectRows(rows, func(r pgx.CollectableRow) (key, error) {
			var k key
			return k, r.Scan(&k.owner, &k.id)
		})
		if err != nil {
			return err
		}
		for _, k := range old {
			c, err := purge(ctx, tx, k.owner, k.id)
			if err != nil {
				return err
			}
			purged = append(purged, c)
		}
		rows, err = tx.Query(ctx,
			`SELECT `+captureColumns+` FROM captures
			 WHERE begun < $1 ORDER BY begun LIMIT $2 FOR UPDATE SKIP LOCKED`, begunBefore, limit)
		if err != nil {
			return err
		}
		dropped, err = pgx.CollectRows(rows, func(r pgx.CollectableRow) (Capture, error) { return scanCapture(r) })
		if err != nil {
			return err
		}
		for _, c := range dropped {
			if c.Version == 0 {
				_, err = tx.Exec(ctx, `DELETE FROM captures WHERE owner = $1 AND id = $2`, c.Owner, c.ID)
			} else {
				_, err = tx.Exec(ctx,
					`UPDATE captures SET pending = NULL, pending_doc = NULL, pending_source = NULL,
					   pending_thumb = NULL, begun = NULL WHERE owner = $1 AND id = $2`, c.Owner, c.ID)
			}
			if err != nil {
				return err
			}
		}
		return nil
	})
	return purged, dropped, err
}
