// Package store keeps the service's records: users, their devices, and
// shares. Postgres in production; an in-memory copy for tests.
package store

import (
	"context"
	"errors"
	"time"
)

var (
	ErrNotFound = errors.New("not found")
	ErrExists   = errors.New("already exists")
	// ErrConflict: a capture's live version isn't the one a new version
	// was based on.
	ErrConflict = errors.New("changed meanwhile")
	// ErrTrashed: the capture is in the trash.
	ErrTrashed = errors.New("in the trash")
	// ErrGone: the capture was deleted for good.
	ErrGone = errors.New("deleted for good")
)

type User struct {
	ID   int64
	Name string
}

// Device is one signed-in app, by the token it was given.
type Device struct {
	ID       int64
	User     string
	Label    string
	Created  time.Time
	LastUsed *time.Time
	Revoked  bool
}

// State is where a share is in its life.
type State string

const (
	// Pending: created, waiting for the app's upload and finalize.
	Pending State = "pending"
	Live    State = "live"
	// Deleted: gone from storage; the record stays so the id isn't reused.
	Deleted State = "deleted"
)

type Share struct {
	ID         string
	Owner      int64
	SecretHash string
	Size       int64
	Type       string
	Created    time.Time
	Expires    *time.Time
	State      State
	// Public: anyone with the link may view it. Otherwise only its owner
	// and recipients can.
	Public bool
}

// Owned is a share as its owner lists it.
type Owned struct {
	Share
	Recipients []string
}

// Received is a share someone else shared with the user.
type Received struct {
	Share
	From string
}

type Store interface {
	AddUser(ctx context.Context, name string) (User, error)
	UserByName(ctx context.Context, name string) (User, error)
	AddDevice(ctx context.Context, user int64, label, tokenHash string) (Device, error)
	RevokeDevice(ctx context.Context, id int64) error
	Devices(ctx context.Context) ([]Device, error)
	// UserByToken finds the user a device token belongs to, unless the
	// device is revoked, and notes the device was used.
	UserByToken(ctx context.Context, tokenHash string) (User, error)

	// CreateShare adds a share, shared with `recipients`.
	CreateShare(ctx context.Context, s Share, recipients []int64) error
	Share(ctx context.Context, id string) (Share, error)
	// Move changes a share's state to `to` if it's in one of `from`, and
	// is ErrNotFound if it isn't.
	Move(ctx context.Context, id string, to State, from ...State) error
	// Shares lists an owner's live shares, newest first.
	Shares(ctx context.Context, owner int64) ([]Owned, error)
	// SetAccess changes whether anyone with the link may view a share,
	// until when, and, unless nil, who it's shared with.
	SetAccess(ctx context.Context, id string, public bool, expires *time.Time, recipients *[]int64) error
	Recipients(ctx context.Context, id string) ([]User, error)
	// SharedWith lists live, unexpired shares shared with `user` at `now`.
	SharedWith(ctx context.Context, user int64, now time.Time) ([]Received, error)
	// Sweep marks up to `limit` shares deleted that are live past expiry
	// at `now`, or pending for longer than `grace`, and returns them.
	Sweep(ctx context.Context, now time.Time, grace time.Duration, limit int) ([]Share, error)

	Accounts
	Captures
}

// Accounts are users who sign in with a password, and their devices.
type Accounts interface {
	// AddAccount adds a user who signs in with a password.
	AddAccount(ctx context.Context, name, passwordHash string) (User, error)
	// Password is user `name` and their password hash, "" for a user made
	// by hand who has none yet.
	Password(ctx context.Context, name string) (User, string, error)
	SetPassword(ctx context.Context, user int64, hash string) error
	// UserDevices lists a user's devices that aren't revoked.
	UserDevices(ctx context.Context, user int64) ([]Device, error)
	// RevokeUserDevice revokes device `id` if it's the user's.
	RevokeUserDevice(ctx context.Context, user, id int64) error
}

// Captures are the captures an account syncs between its devices.
type Captures interface {
	// BeginCapture starts uploading a new version of capture `id` on top
	// of version `base` (0 for a new capture), and returns the new
	// version's number and the number of any upload it replaced (0 if
	// none). ErrConflict when `base` isn't the live version, ErrTrashed
	// or ErrGone when it can't change.
	BeginCapture(ctx context.Context, owner int64, id string, base int64, taken time.Time, sizes Sizes, now time.Time) (version, replaced int64, err error)
	// FinishCapture makes the version being uploaded live, if it's
	// `version`, and returns the capture as it was before.
	FinishCapture(ctx context.Context, owner int64, id string, version int64, now time.Time) (Capture, error)
	Capture(ctx context.Context, owner int64, id string) (Capture, error)
	// Changes lists an owner's captures changed after `since`, oldest
	// change first, at most `limit`; never-finished ones are left out.
	Changes(ctx context.Context, owner int64, since int64, limit int) ([]Capture, error)
	TrashCapture(ctx context.Context, owner int64, id string, now time.Time) error
	RestoreCapture(ctx context.Context, owner int64, id string) error
	// PurgeCapture keeps only the record of a capture, so devices learn
	// it's gone, and returns it as it was, so its objects can go.
	PurgeCapture(ctx context.Context, owner int64, id string) (Capture, error)
	// Usage is how many bytes an owner's captures take, uploads included.
	Usage(ctx context.Context, owner int64) (int64, error)
	// SweepCaptures purges up to `limit` captures trashed before
	// `trashedBefore`, and drops uploads begun before `begunBefore`,
	// returning both as they were.
	SweepCaptures(ctx context.Context, trashedBefore, begunBefore time.Time, limit int) (purged, dropped []Capture, err error)
}

// Sizes are the bytes of a capture's three parts.
type Sizes struct {
	Doc, Source, Thumb int64
}

func (s Sizes) Total() int64 { return s.Doc + s.Source + s.Thumb }

// Capture is one synced capture: its live version, and the one being
// uploaded.
type Capture struct {
	ID    string
	Owner int64
	// Version is the live version, 0 until the first is finished.
	Version int64
	// Seq orders an account's changes; it moves on with each one.
	Seq     int64
	Taken   time.Time
	Updated time.Time
	Sizes   Sizes
	// Pending is the version being uploaded, 0 for none.
	Pending      int64
	PendingSizes Sizes
	Begun        *time.Time
	Trashed      *time.Time
	// Gone: deleted for good; only the record is left.
	Gone bool
}
