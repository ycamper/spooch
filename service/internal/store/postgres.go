package store

import (
	"context"
	_ "embed"
	"errors"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"
)

//go:embed schema.sql
var schema string

// Postgres is the production store.
type Postgres struct {
	pool *pgxpool.Pool
}

// Open connects to `url` and makes sure the tables exist.
func Open(ctx context.Context, url string) (*Postgres, error) {
	pool, err := pgxpool.New(ctx, url)
	if err != nil {
		return nil, err
	}
	if _, err := pool.Exec(ctx, schema); err != nil {
		pool.Close()
		return nil, err
	}
	return &Postgres{pool: pool}, nil
}

func (p *Postgres) Close() { p.pool.Close() }

// notFound turns pgx's no-rows error into ErrNotFound.
func notFound(err error) error {
	if errors.Is(err, pgx.ErrNoRows) {
		return ErrNotFound
	}
	return err
}

// exists turns a unique-key violation into ErrExists.
func exists(err error) error {
	var pg *pgconn.PgError
	if errors.As(err, &pg) && pg.Code == "23505" {
		return ErrExists
	}
	return err
}

// changed is ErrNotFound when an update touched no rows.
func changed(tag pgconn.CommandTag, err error) error {
	if err == nil && tag.RowsAffected() == 0 {
		return ErrNotFound
	}
	return err
}

func (p *Postgres) AddUser(ctx context.Context, name string) (User, error) {
	u := User{Name: name}
	err := p.pool.QueryRow(ctx, `INSERT INTO users (name) VALUES ($1) RETURNING id`, name).Scan(&u.ID)
	return u, exists(err)
}

func (p *Postgres) UserByName(ctx context.Context, name string) (User, error) {
	u := User{Name: name}
	err := p.pool.QueryRow(ctx, `SELECT id FROM users WHERE name = $1`, name).Scan(&u.ID)
	return u, notFound(err)
}

func (p *Postgres) AddDevice(ctx context.Context, user int64, label, tokenHash string) (Device, error) {
	d := Device{Label: label}
	err := p.pool.QueryRow(ctx,
		`INSERT INTO devices (user_id, label, token_hash) VALUES ($1, $2, $3)
		 RETURNING id, created, (SELECT name FROM users WHERE id = $1)`,
		user, label, tokenHash).Scan(&d.ID, &d.Created, &d.User)
	return d, exists(err)
}

func (p *Postgres) RevokeDevice(ctx context.Context, id int64) error {
	return changed(p.pool.Exec(ctx, `UPDATE devices SET revoked = true WHERE id = $1`, id))
}

func (p *Postgres) Devices(ctx context.Context) ([]Device, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT d.id, u.name, d.label, d.created, d.last_used, d.revoked
		 FROM devices d JOIN users u ON u.id = d.user_id ORDER BY d.id`)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Device, error) {
		var d Device
		err := r.Scan(&d.ID, &d.User, &d.Label, &d.Created, &d.LastUsed, &d.Revoked)
		return d, err
	})
}

func (p *Postgres) UserByToken(ctx context.Context, tokenHash string) (User, error) {
	var (
		u      User
		device int64
		stale  bool
	)
	err := p.pool.QueryRow(ctx,
		`SELECT u.id, u.name, d.id, d.last_used IS NULL OR d.last_used < now() - interval '5 minutes'
		 FROM devices d JOIN users u ON u.id = d.user_id
		 WHERE d.token_hash = $1 AND NOT d.revoked`, tokenHash).Scan(&u.ID, &u.Name, &device, &stale)
	if err != nil {
		return User{}, notFound(err)
	}
	// Noted at most every few minutes, so most requests only read.
	if stale {
		_, err = p.pool.Exec(ctx, `UPDATE devices SET last_used = now() WHERE id = $1`, device)
	}
	return u, err
}

func (p *Postgres) CreateShare(ctx context.Context, s Share, recipients []int64) error {
	return pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		_, err := tx.Exec(ctx,
			`INSERT INTO shares (id, owner, secret_hash, size, type, created, expires, state, public)
			 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
			s.ID, s.Owner, s.SecretHash, s.Size, s.Type, s.Created, s.Expires, s.State, s.Public)
		if err != nil {
			return exists(err)
		}
		return setRecipients(ctx, tx, s.ID, recipients)
	})
}

func setRecipients(ctx context.Context, tx pgx.Tx, id string, users []int64) error {
	if _, err := tx.Exec(ctx, `DELETE FROM share_recipients WHERE share_id = $1`, id); err != nil {
		return err
	}
	_, err := tx.Exec(ctx,
		`INSERT INTO share_recipients (share_id, user_id) SELECT $1, unnest($2::bigint[])`, id, users)
	return err
}

const shareColumns = `s.id, s.owner, s.secret_hash, s.size, s.type, s.created, s.expires, s.state, s.public`

func scanShare(r pgx.Row, extra ...any) (Share, error) {
	var s Share
	dest := append([]any{&s.ID, &s.Owner, &s.SecretHash, &s.Size, &s.Type, &s.Created, &s.Expires, &s.State, &s.Public}, extra...)
	err := r.Scan(dest...)
	return s, err
}

func (p *Postgres) Share(ctx context.Context, id string) (Share, error) {
	s, err := scanShare(p.pool.QueryRow(ctx, `SELECT `+shareColumns+` FROM shares s WHERE id = $1`, id))
	return s, notFound(err)
}

func (p *Postgres) Move(ctx context.Context, id string, to State, from ...State) error {
	states := make([]string, len(from))
	for i, f := range from {
		states[i] = string(f)
	}
	return changed(p.pool.Exec(ctx,
		`UPDATE shares SET state = $2 WHERE id = $1 AND state = ANY($3)`, id, string(to), states))
}

func (p *Postgres) SharesSince(ctx context.Context, owner int64, since time.Time) (int, error) {
	var n int
	err := p.pool.QueryRow(ctx,
		`SELECT count(*) FROM shares WHERE owner = $1 AND created >= $2`, owner, since).Scan(&n)
	return n, err
}

func (p *Postgres) Shares(ctx context.Context, owner int64) ([]Owned, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT `+shareColumns+`,
		   COALESCE(array_agg(u.name ORDER BY u.name) FILTER (WHERE u.name IS NOT NULL), '{}')
		 FROM shares s
		 LEFT JOIN share_recipients r ON r.share_id = s.id
		 LEFT JOIN users u ON u.id = r.user_id
		 WHERE s.owner = $1 AND s.state = 'live'
		 GROUP BY s.id ORDER BY s.created DESC`, owner)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Owned, error) {
		var names []string
		s, err := scanShare(r, &names)
		return Owned{Share: s, Recipients: names}, err
	})
}

func (p *Postgres) SetAccess(ctx context.Context, id string, public bool, expires *time.Time, recipients *[]int64) error {
	return pgx.BeginFunc(ctx, p.pool, func(tx pgx.Tx) error {
		err := changed(tx.Exec(ctx, `UPDATE shares SET public = $2, expires = $3 WHERE id = $1`, id, public, expires))
		if err != nil || recipients == nil {
			return err
		}
		return setRecipients(ctx, tx, id, *recipients)
	})
}

func (p *Postgres) Recipients(ctx context.Context, id string) ([]User, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT u.id, u.name FROM share_recipients r JOIN users u ON u.id = r.user_id
		 WHERE r.share_id = $1 ORDER BY u.name`, id)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (User, error) {
		var u User
		err := r.Scan(&u.ID, &u.Name)
		return u, err
	})
}

func (p *Postgres) SharedWith(ctx context.Context, user int64, now time.Time) ([]Received, error) {
	rows, err := p.pool.Query(ctx,
		`SELECT `+shareColumns+`, u.name FROM share_recipients r
		 JOIN shares s ON s.id = r.share_id JOIN users u ON u.id = s.owner
		 WHERE r.user_id = $1 AND s.state = 'live' AND (s.expires IS NULL OR s.expires > $2)
		 ORDER BY s.created DESC`, user, now)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Received, error) {
		var from string
		s, err := scanShare(r, &from)
		return Received{Share: s, From: from}, err
	})
}

// staleWhere is what Sweep clears. It's checked again row by row as each
// is updated, so a share extended or finalized meanwhile is left alone.
const staleWhere = `((state = 'live' AND expires < $1) OR (state = 'pending' AND created < $2))`

func (p *Postgres) Sweep(ctx context.Context, now time.Time, grace time.Duration, limit int) ([]Share, error) {
	rows, err := p.pool.Query(ctx,
		`UPDATE shares s SET state = 'deleted'
		 WHERE `+staleWhere+` AND id IN (SELECT id FROM shares WHERE `+staleWhere+` LIMIT $3)
		 RETURNING `+shareColumns, now, now.Add(-grace), limit)
	if err != nil {
		return nil, err
	}
	return pgx.CollectRows(rows, func(r pgx.CollectableRow) (Share, error) { return scanShare(r) })
}
